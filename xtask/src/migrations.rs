use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use sqlparser::ast::{
    AlterViewOperation, FromTable, GrantObjects, ObjectName, ObjectNamePart, ObjectType,
    SchemaName, Set, Statement, TableFactor, TableObject, TableWithJoins,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::{rls, session};

const RULE: &str = "Every object lives in the schema of its bounded context, like catalog.item, and nothing goes into public. A bounded context is a module of crates/db, created with cargo run -p xtask -- new-context <name>.";

enum Target<'a> {
    Schema(String, &'a ObjectName),
    Object(String, &'a ObjectName),
}

pub(crate) fn check(workspace: &Path, out: &mut impl Write) -> Result<bool, String> {
    let mut found = session::violations(&session::sources(workspace)?);
    let directory = workspace.join("crates").join("db").join("migrations");
    if directory.is_dir() {
        let contexts = contexts(&workspace.join("crates").join("db").join("src"))?;
        let mut files = Vec::new();
        for path in sorted_entries(&directory)? {
            if path.extension().is_none_or(|extension| extension != "sql") {
                continue;
            }
            let sql = fs::read_to_string(&path)
                .map_err(|error| format!("{}: cannot read: {error}", path.display()))?;
            let file = path
                .strip_prefix(workspace)
                .unwrap_or(&path)
                .display()
                .to_string();
            found.extend(violations(&file, &sql, &contexts));
            files.push((file, sql));
        }
        found.extend(rls::violations(&files));
    }
    for violation in &found {
        writeln!(out, "{violation}")
            .map_err(|error| format!("cannot write the report: {error}"))?;
    }
    Ok(found.is_empty())
}

fn contexts(db_src: &Path) -> Result<BTreeSet<String>, String> {
    Ok(sorted_entries(db_src)?
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter_map(|path| path.file_stem().and_then(|stem| stem.to_str()))
        .filter(|stem| *stem != "lib")
        .map(str::to_owned)
        .collect())
}

pub(crate) fn sorted_entries(directory: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut paths = fs::read_dir(directory)
        .map_err(|error| format!("{}: cannot read: {error}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: cannot read: {error}", directory.display()))?;
    paths.sort();
    Ok(paths)
}

pub(crate) fn violations(file: &str, sql: &str, contexts: &BTreeSet<String>) -> Vec<String> {
    let statements = match Parser::parse_sql(&PostgreSqlDialect {}, sql) {
        Ok(statements) => statements,
        Err(error) => {
            return vec![format!(
                "{file}: not valid PostgreSQL ({error}). check-migrations parses every migration to prove it keeps to the schema rule, so write it in plain PostgreSQL the parser accepts. {RULE}"
            )];
        }
    };
    let mut found = Vec::new();
    for statement in &statements {
        match targets(statement) {
            Ok(targets) => found.extend(
                targets
                    .iter()
                    .filter_map(|target| target_violation(target, contexts))
                    .map(|violation| format!("{file}: {violation}")),
            ),
            Err(violation) => found.push(format!("{file}: {violation}")),
        }
    }
    found
}

fn targets(statement: &Statement) -> Result<Vec<Target<'_>>, String> {
    let object = |label: &str, name| Ok(vec![Target::Object(label.to_owned(), name)]);
    match statement {
        Statement::CreateSchema {
            schema_name: SchemaName::Simple(name) | SchemaName::NamedAuthorization(name, _),
            ..
        } => Ok(vec![Target::Schema("CREATE SCHEMA".to_owned(), name)]),
        Statement::CreateTable(table) => object("CREATE TABLE", &table.name),
        Statement::CreateIndex(index) => object("CREATE INDEX on", &index.table_name),
        Statement::CreateType { name, .. } => object("CREATE TYPE", name),
        Statement::CreateView(view) => object("CREATE VIEW", &view.name),
        Statement::CreateFunction(function) => object("CREATE FUNCTION", &function.name),
        Statement::CreateSequence { name, .. } => object("CREATE SEQUENCE", name),
        Statement::CreateTrigger(trigger) => object("CREATE TRIGGER on", &trigger.table_name),
        Statement::AlterTable(table) => object("ALTER TABLE", &table.name),
        Statement::AlterIndex { name, .. } => object("ALTER INDEX", name),
        Statement::AlterView(view) => {
            let mut targets = vec![Target::Object("ALTER VIEW".to_owned(), &view.name)];
            if let AlterViewOperation::SetSchema { schema_name } = &view.operation {
                targets.push(Target::Schema(
                    format!("ALTER VIEW {} SET SCHEMA", view.name),
                    schema_name,
                ));
            }
            Ok(targets)
        }
        Statement::AlterType(kind) => object("ALTER TYPE", &kind.name),
        Statement::AlterFunction(alter) => object("ALTER FUNCTION", &alter.function.name),
        Statement::Drop {
            object_type, names, ..
        } => Ok(names
            .iter()
            .map(|name| match object_type {
                ObjectType::Schema => Target::Schema("DROP SCHEMA".to_owned(), name),
                _ => Target::Object(format!("DROP {object_type}"), name),
            })
            .collect()),
        Statement::Truncate(truncate) => Ok(truncate
            .table_names
            .iter()
            .map(|target| Target::Object("TRUNCATE".to_owned(), &target.name))
            .collect()),
        Statement::Insert(insert) => match &insert.table {
            TableObject::TableName(name) => object("INSERT INTO", name),
            TableObject::TableFunction(_) | TableObject::TableQuery(_) => {
                Err(not_allowed("INSERT INTO anything but one named table"))
            }
        },
        Statement::Update(update) => relations("UPDATE", std::slice::from_ref(&update.table)),
        Statement::Delete(delete) => match &delete.from {
            FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables)
                if delete.tables.is_empty() =>
            {
                relations("DELETE FROM", tables)
            }
            FromTable::WithFromKeyword(_) | FromTable::WithoutKeyword(_) => {
                Err(not_allowed("a multi-table DELETE"))
            }
        },
        Statement::Set(set) => set_variables(set),
        Statement::CreatePolicy(policy) => object("CREATE POLICY on", &policy.table_name),
        Statement::Grant(grant) => grant_targets("GRANT", grant.objects.as_ref()),
        Statement::Revoke(revoke) => grant_targets("REVOKE", revoke.objects.as_ref()),
        _ => Err(not_allowed(&statement_kind(statement))),
    }
}

fn grant_targets<'a>(
    label: &str,
    objects: Option<&'a GrantObjects>,
) -> Result<Vec<Target<'a>>, String> {
    let named = |kind: &str, names: &'a [ObjectName]| {
        Ok(names
            .iter()
            .map(|name| Target::Object(format!("{label} on {kind}"), name))
            .collect())
    };
    match objects {
        Some(GrantObjects::Tables(names)) => named("table", names),
        Some(GrantObjects::Views(names)) => named("view", names),
        Some(GrantObjects::Sequences(names)) => named("sequence", names),
        Some(GrantObjects::Schemas(names)) => Ok(names
            .iter()
            .map(|name| Target::Schema(format!("{label} on schema"), name))
            .collect()),
        Some(GrantObjects::Functions(functions)) => Ok(functions
            .iter()
            .map(|function| Target::Object(format!("{label} on function"), &function.name))
            .collect()),
        Some(_) => Err(not_allowed(&format!(
            "{label} on anything but named tables, views, sequences, functions or schemas"
        ))),
        None => Err(not_allowed(&format!("{label} of a role"))),
    }
}

fn relations<'a>(label: &str, tables: &'a [TableWithJoins]) -> Result<Vec<Target<'a>>, String> {
    tables
        .iter()
        .map(|table| match &table.relation {
            TableFactor::Table { name, .. } if table.joins.is_empty() => {
                Ok(Target::Object(label.to_owned(), name))
            }
            _ => Err(not_allowed(&format!(
                "{label} on anything but one named table"
            ))),
        })
        .collect()
}

fn set_variables(set: &Set) -> Result<Vec<Target<'_>>, String> {
    let variables: Vec<&ObjectName> = match set {
        Set::SingleAssignment { variable, .. } => vec![variable],
        Set::ParenthesizedAssignments { variables, .. } => variables.iter().collect(),
        Set::MultipleAssignments { assignments } => assignments
            .iter()
            .map(|assignment| &assignment.name)
            .collect(),
        _ => return Err(not_allowed(&format!("SET {}", statement_kind(set)))),
    };
    if variables
        .iter()
        .any(|variable| parts(variable).is_some_and(|parts| parts == ["search_path"]))
    {
        Err(format!(
            "SET search_path: a migration does not change the search_path, because it makes unqualified names land in whatever schema it names. Drop the SET and qualify every name with its context schema. {RULE}"
        ))
    } else {
        Ok(Vec::new())
    }
}

fn target_violation(target: &Target<'_>, contexts: &BTreeSet<String>) -> Option<String> {
    match target {
        Target::Schema(label, name) => match parts(name).as_deref() {
            Some([schema]) => schema_violation(&format!("{label} {name}"), schema, contexts),
            _ => Some(format!(
                "{label} {name}: a schema name has one part, the bounded context, like catalog. {RULE}"
            )),
        },
        Target::Object(label, name) => match parts(name).as_deref() {
            Some([_]) => Some(format!(
                "{label} {name}: the name is not schema-qualified, so it resolves to public. Write <context>.{name}. {RULE}"
            )),
            Some([schema, _]) => schema_violation(&format!("{label} {name}"), schema, contexts),
            _ => Some(format!(
                "{label} {name}: write the name as <context>.<name>, with exactly two parts. {RULE}"
            )),
        },
    }
}

fn schema_violation(at: &str, schema: &str, contexts: &BTreeSet<String>) -> Option<String> {
    if schema == "public" {
        Some(format!(
            "{at}: nothing goes into public. Put it in the schema of its bounded context. {RULE}"
        ))
    } else if contexts.contains(schema) {
        None
    } else {
        Some(format!(
            "{at}: schema {schema} is not a bounded context; run cargo run -p xtask -- new-context {schema}, or use the schema of an existing context. {RULE}"
        ))
    }
}

pub(crate) fn parts(name: &ObjectName) -> Option<Vec<String>> {
    name.0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) if ident.quote_style.is_some() => {
                Some(ident.value.clone())
            }
            ObjectNamePart::Identifier(ident) => Some(ident.value.to_lowercase()),
            ObjectNamePart::Function(_) => None,
        })
        .collect()
}

fn statement_kind(value: &impl std::fmt::Debug) -> String {
    format!("{value:?}")
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

fn not_allowed(kind: &str) -> String {
    format!(
        "{kind} is not allowed in a migration. check-migrations allows CREATE SCHEMA, CREATE TABLE, INDEX, TYPE, VIEW, FUNCTION, SEQUENCE, TRIGGER and POLICY, ALTER TABLE, INDEX, VIEW, TYPE and FUNCTION, DROP, TRUNCATE, INSERT, UPDATE, DELETE, GRANT, REVOKE and SET, each on a context schema, so it can prove every object lands in a context schema. If the migration needs {kind}, ask the user to extend check-migrations. {RULE}"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    const FILE: &str = "crates/db/migrations/0001_test.sql";

    fn found(sql: &str) -> Vec<String> {
        let contexts = BTreeSet::from(["catalog".to_owned(), "billing".to_owned()]);
        super::violations(FILE, sql, &contexts)
    }

    fn assert_one(sql: &str, expected: &str) {
        let found = found(sql);
        assert_eq!(found.len(), 1, "{sql}: {found:?}");
        assert!(
            found[0].starts_with(&format!("{FILE}: {expected}")),
            "{sql}: {found:?}"
        );
        assert!(found[0].ends_with(super::RULE), "{sql}: {found:?}");
    }

    #[test]
    fn a_migration_that_keeps_to_its_context_passes() {
        let sql = r"
            CREATE SCHEMA IF NOT EXISTS catalog;
            SET LOCAL lock_timeout = '5s';
            CREATE TYPE catalog.item_state AS ENUM ('draft', 'final');
            CREATE SEQUENCE catalog.item_number;
            CREATE TABLE catalog.item (
                id uuid PRIMARY KEY DEFAULT uuidv7(),
                invoice_id uuid REFERENCES billing.invoice (id),
                state catalog.item_state NOT NULL
            );
            CREATE INDEX item_state ON catalog.item (state);
            ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY;
            CREATE POLICY admin_access ON catalog.item TO app_admin USING (true) WITH CHECK (true);
            GRANT USAGE ON SCHEMA catalog TO app_api;
            GRANT SELECT, INSERT ON catalog.item TO app_api;
            REVOKE DELETE ON catalog.item FROM app_api;
            CREATE VIEW catalog.final_item AS SELECT id FROM catalog.item WHERE state = 'final';
            CREATE FUNCTION catalog.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
            CREATE TRIGGER touch BEFORE UPDATE ON catalog.item FOR EACH ROW EXECUTE FUNCTION catalog.touch();
            ALTER FUNCTION catalog.touch() OWNER TO app_view;
            ALTER TABLE catalog.item ADD COLUMN note text;
            ALTER INDEX catalog.item_state RENAME TO item_by_state;
            INSERT INTO catalog.item (state) SELECT 'draft' FROM billing.invoice;
            UPDATE catalog.item SET note = i.number FROM billing.invoice i WHERE i.id = invoice_id;
            DELETE FROM catalog.item USING billing.invoice i WHERE i.id = invoice_id;
            TRUNCATE catalog.item;
            DROP VIEW catalog.final_item;
            DROP TABLE Catalog.item;
        ";
        assert_eq!(found(sql), Vec::<String>::new());
    }

    #[test]
    fn an_empty_migration_passes() {
        assert_eq!(found(""), Vec::<String>::new());
    }

    #[test]
    fn unparseable_sql_is_reported() {
        let found = found("CREATE TABLE catalog.x (id uuid");
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with(&format!("{FILE}: not valid PostgreSQL (")));
    }

    #[test]
    fn every_object_must_be_schema_qualified() {
        for (sql, label) in [
            ("CREATE TABLE item (id uuid)", "CREATE TABLE item"),
            ("CREATE INDEX i ON item (id)", "CREATE INDEX on item"),
            ("CREATE TYPE state AS ENUM ('a')", "CREATE TYPE state"),
            ("CREATE VIEW v AS SELECT 1", "CREATE VIEW v"),
            (
                "CREATE FUNCTION f() RETURNS int LANGUAGE sql AS $$ SELECT 1 $$",
                "CREATE FUNCTION f",
            ),
            ("CREATE SEQUENCE s", "CREATE SEQUENCE s"),
            (
                "CREATE TRIGGER t BEFORE UPDATE ON item FOR EACH ROW EXECUTE FUNCTION catalog.touch()",
                "CREATE TRIGGER on item",
            ),
            (
                "ALTER TABLE item ENABLE ROW LEVEL SECURITY",
                "ALTER TABLE item",
            ),
            (
                "CREATE POLICY admin_access ON item TO app_admin USING (true) WITH CHECK (true)",
                "CREATE POLICY on item",
            ),
            ("GRANT SELECT ON item TO app_api", "GRANT on table item"),
            ("UPDATE item SET title = 'x'", "UPDATE item"),
        ] {
            assert_one(
                sql,
                &format!("{label}: the name is not schema-qualified, so it resolves to public."),
            );
        }
    }

    #[test]
    fn nothing_goes_into_public() {
        for (sql, label) in [
            (
                "CREATE TABLE public.item (id uuid)",
                "CREATE TABLE public.item",
            ),
            (
                "CREATE TABLE PUBLIC.item (id uuid)",
                "CREATE TABLE PUBLIC.item",
            ),
            (
                "CREATE INDEX i ON public.item (id)",
                "CREATE INDEX on public.item",
            ),
            ("CREATE SCHEMA IF NOT EXISTS public", "CREATE SCHEMA public"),
            (
                "ALTER TABLE public.item ENABLE ROW LEVEL SECURITY",
                "ALTER TABLE public.item",
            ),
            (
                "GRANT SELECT ON public.item TO app_api",
                "GRANT on table public.item",
            ),
            (
                "GRANT USAGE ON SCHEMA public TO app_api",
                "GRANT on schema public",
            ),
            ("DROP TABLE public.item", "DROP TABLE public.item"),
            ("TRUNCATE public.item", "TRUNCATE public.item"),
            (
                "INSERT INTO public.item (id) VALUES (uuidv7())",
                "INSERT INTO public.item",
            ),
            ("DELETE FROM public.item", "DELETE FROM public.item"),
            (
                "ALTER FUNCTION public.f() OWNER TO app_view",
                "ALTER FUNCTION public.f",
            ),
        ] {
            assert_one(sql, &format!("{label}: nothing goes into public."));
        }
    }

    #[test]
    fn nothing_moves_into_public_and_no_schema_is_dropped_outside_a_context() {
        for (sql, expected) in [
            (
                "ALTER VIEW catalog.final_item SET SCHEMA public",
                "ALTER VIEW catalog.final_item SET SCHEMA public: nothing goes into public.",
            ),
            (
                "ALTER VIEW catalog.final_item SET SCHEMA accounts",
                "ALTER VIEW catalog.final_item SET SCHEMA accounts: schema accounts is not a bounded context",
            ),
            (
                "DROP SCHEMA public",
                "DROP SCHEMA public: nothing goes into public.",
            ),
            (
                "DROP SCHEMA accounts",
                "DROP SCHEMA accounts: schema accounts is not a bounded context",
            ),
        ] {
            assert_one(sql, expected);
        }
        assert_eq!(found("DROP SCHEMA catalog"), Vec::<String>::new());
    }

    #[test]
    fn moving_a_table_or_type_to_another_schema_is_unparseable_and_fails() {
        for sql in [
            "ALTER TABLE catalog.item SET SCHEMA public",
            "ALTER TYPE catalog.item_state SET SCHEMA public",
        ] {
            let found = found(sql);
            assert_eq!(found.len(), 1, "{sql}: {found:?}");
            assert!(
                found[0].starts_with(&format!("{FILE}: not valid PostgreSQL (")),
                "{sql}: {found:?}"
            );
            assert!(found[0].ends_with(super::RULE), "{sql}: {found:?}");
        }
    }

    #[test]
    fn a_quoted_schema_is_compared_exactly() {
        assert_one(
            r#"CREATE TABLE "Catalog".x (id uuid)"#,
            "CREATE TABLE \"Catalog\".x: schema Catalog is not a bounded context",
        );
    }

    #[test]
    fn set_search_path_is_reported() {
        assert_one("SET search_path TO catalog", "SET search_path: ");
        assert_one(
            "SET LOCAL search_path = catalog, public",
            "SET search_path: ",
        );
    }

    #[test]
    fn an_unknown_schema_names_the_generator() {
        assert_one(
            "CREATE TABLE accounts.deletion (id uuid)",
            "CREATE TABLE accounts.deletion: schema accounts is not a bounded context; run cargo run -p xtask -- new-context accounts",
        );
        assert_one(
            "CREATE SCHEMA accounts",
            "CREATE SCHEMA accounts: schema accounts is not a bounded context; run cargo run -p xtask -- new-context accounts",
        );
    }

    #[test]
    fn a_name_with_more_than_two_parts_is_reported() {
        assert_one(
            "CREATE TABLE app.catalog.x (id uuid)",
            "CREATE TABLE app.catalog.x: write the name as <context>.<name>",
        );
    }

    #[test]
    fn statements_the_check_cannot_judge_are_reported() {
        for (sql, kind) in [
            ("CREATE EXTENSION pgcrypto", "CreateExtension"),
            ("CREATE ROLE app_api NOLOGIN", "CreateRole"),
            ("COMMENT ON TABLE catalog.item IS 'x'", "Comment"),
            ("GRANT app_admin TO reader", "GrantRole"),
            (
                "GRANT SELECT ON ALL TABLES IN SCHEMA catalog TO reader",
                "GRANT on anything but named tables, views, sequences, functions or schemas",
            ),
            ("DROP POLICY org_scope ON catalog.x", "DropPolicy"),
            (
                "ALTER POLICY org_scope ON catalog.x USING (true)",
                "AlterPolicy",
            ),
            (
                "ALTER DEFAULT PRIVILEGES IN SCHEMA catalog GRANT SELECT ON TABLES TO reader",
                "AlterDefaultPrivileges",
            ),
            ("SELECT 1", "Query"),
            ("SET ROLE app_admin", "SET SetRole"),
        ] {
            assert_one(sql, &format!("{kind} is not allowed in a migration."));
        }
    }

    #[test]
    fn every_statement_is_reported() {
        let found = found(
            "CREATE TABLE catalog.ok (id uuid); ALTER TABLE public.item ADD COLUMN a text; CREATE TABLE b (id uuid);",
        );
        assert_eq!(found.len(), 2, "{found:?}");
    }

    fn workspace(test: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("xtask-migrations-{}-{test}", std::process::id()));
        let src = root.join("crates/db/src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "pub mod catalog;\n").unwrap();
        fs::write(src.join("catalog.rs"), "").unwrap();
        root
    }

    #[test]
    fn a_workspace_without_migrations_passes() {
        let root = workspace("none");
        let mut out = Vec::new();
        assert_eq!(super::check(&root, &mut out), Ok(true));
        assert_eq!(out, Vec::<u8>::new());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_files_are_checked_against_the_db_modules() {
        let root = workspace("files");
        let migrations = root.join("crates/db/migrations");
        fs::create_dir_all(&migrations).unwrap();
        fs::write(
            migrations.join("0001_ok.sql"),
            "CREATE TABLE catalog.x (id uuid); ALTER TABLE catalog.x ENABLE ROW LEVEL SECURITY; ALTER TABLE catalog.x FORCE ROW LEVEL SECURITY;",
        )
        .unwrap();
        fs::write(
            migrations.join("0002_lib.sql"),
            "CREATE TABLE lib.x (id uuid);",
        )
        .unwrap();
        fs::write(
            migrations.join("0003_open.sql"),
            "CREATE TABLE catalog.open (id uuid);",
        )
        .unwrap();
        fs::write(migrations.join("notes.txt"), "CREATE TABLE x (id uuid);").unwrap();
        let mut out = Vec::new();
        assert_eq!(super::check(&root, &mut out), Ok(false));
        let report = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 3, "{report}");
        assert!(lines[0].starts_with(
            "crates/db/migrations/0002_lib.sql: CREATE TABLE lib.x: schema lib is not a bounded context"
        ));
        assert!(lines[1].starts_with(
            "crates/db/migrations/0003_open.sql: CREATE TABLE catalog.open: the same migration needs ALTER TABLE catalog.open ENABLE ROW LEVEL SECURITY"
        ));
        assert!(lines[2].starts_with(
            "crates/db/migrations/0002_lib.sql: CREATE TABLE lib.x: the same migration needs"
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
