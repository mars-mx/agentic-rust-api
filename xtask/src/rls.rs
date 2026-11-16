use std::collections::BTreeMap;

use sqlparser::ast::{
    Action, AlterFunction, AlterFunctionAction, AlterFunctionOperation, AlterTableOperation,
    AlterViewOperation, BinaryOperator, CreateFunction, CreateFunctionBody, CreatePolicy,
    CreatePolicyCommand, CreatePolicyType, CreateTableOptions, CreateView, Expr, FunctionSecurity,
    FunctionSetValue, GrantObjects, Grantee, GranteeName, GranteesType, ObjectName, Owner,
    Privileges, Query, SetExpr, SqlOption, Statement, TableFactor, Value,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::migrations::parts;

pub(crate) const RULE: &str = "Every table has row level security, enabled and forced in the migration that creates it. A policy is named after its class and goes to the one group role of that class: org_scope, parent_scope or shared_read to app_api, admin_access to app_admin, view_read to app_view, at most one per table and group. A table without a policy for a group is visible to no role in it. A view is a security_invoker view, or a gate view: owned by app_view in the migration that creates it, with an organization filter in its WHERE. A table behind a gate view has no shared_read for app_api, so the API reads only its own rows directly and the shared ones through the gate. A SECURITY DEFINER function is only a gate function: LANGUAGE sql, SET search_path = pg_catalog, pg_temp, one SELECT with the organization filter, EXECUTE revoked from PUBLIC and granted to app_api, then owned by app_view.";

const API: &str = "app_api";
const GATE: &str = "app_view";
const ADMIN: &str = "app_admin";

const ORGANIZATION: &str = "current_setting('app.organization_id')";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    Organization,
    Parent,
    Everything,
}

struct Class {
    name: &'static str,
    group: &'static str,
    select_only: bool,
    check: Check,
}

const CLASSES: [Class; 5] = [
    Class {
        name: "org_scope",
        group: API,
        select_only: false,
        check: Check::Organization,
    },
    Class {
        name: "parent_scope",
        group: API,
        select_only: false,
        check: Check::Parent,
    },
    Class {
        name: "shared_read",
        group: API,
        select_only: true,
        check: Check::Everything,
    },
    Class {
        name: "admin_access",
        group: ADMIN,
        select_only: false,
        check: Check::Everything,
    },
    Class {
        name: "view_read",
        group: GATE,
        select_only: true,
        check: Check::Everything,
    },
];

enum View {
    Invoker,
    Gate {
        kind: &'static str,
        file: String,
        label: String,
        owned: bool,
        revoked: bool,
    },
}

struct Table {
    created_in: Option<String>,
    label: String,
    enabled: bool,
    forced: bool,
    classes: BTreeMap<&'static str, (&'static str, String)>,
}

impl Table {
    fn new(created_in: Option<String>, name: &ObjectName) -> Self {
        Self {
            created_in,
            label: name.to_string(),
            enabled: false,
            forced: false,
            classes: BTreeMap::new(),
        }
    }
}

pub(crate) fn violations(files: &[(String, String)]) -> Vec<String> {
    let mut tables: BTreeMap<String, Table> = BTreeMap::new();
    let mut views: BTreeMap<String, View> = BTreeMap::new();
    let mut functions: BTreeMap<String, View> = BTreeMap::new();
    let mut found = Vec::new();
    for (file, sql) in files {
        let Ok(statements) = Parser::parse_sql(&PostgreSqlDialect {}, sql) else {
            continue;
        };
        for statement in &statements {
            let report = |violation: String| format!("{file}: {violation} {RULE}");
            match statement {
                Statement::CreateTable(table) => {
                    tables.insert(
                        key(&table.name),
                        Table::new(Some(file.clone()), &table.name),
                    );
                }
                Statement::AlterTable(alter) => {
                    for operation in &alter.operations {
                        if let Some(violation) =
                            alter_violation(file, &alter.name, operation, &mut tables)
                        {
                            found.push(report(violation));
                        }
                    }
                }
                Statement::CreatePolicy(policy) => {
                    if let Some(violation) = policy_violation(file, policy, &mut tables) {
                        found.push(report(violation));
                    }
                }
                Statement::CreateView(view) => match view_kind(file, view) {
                    Ok(kind) => {
                        views.insert(key(&view.name), kind);
                    }
                    Err(violation) => found.push(report(violation)),
                },
                Statement::AlterView(alter) => {
                    if let Some(violation) =
                        alter_view_violation(file, &alter.name, &alter.operation, &mut views)
                    {
                        found.push(report(violation));
                    }
                }
                Statement::CreateFunction(function)
                    if function.security == Some(FunctionSecurity::Definer) =>
                {
                    if let Some(violation) = define_function(file, function, &mut functions) {
                        found.push(report(violation));
                    }
                }
                Statement::Grant(grant) => {
                    if let Some(violation) = privilege_violation(
                        "GRANT",
                        &grant.privileges,
                        grant.objects.as_ref(),
                        &grant.grantees,
                        &mut functions,
                    ) {
                        found.push(report(violation));
                    }
                }
                Statement::Revoke(revoke) if !revoke.grant_option_for => {
                    if let Some(violation) = privilege_violation(
                        "REVOKE",
                        &revoke.privileges,
                        revoke.objects.as_ref(),
                        &revoke.grantees,
                        &mut functions,
                    ) {
                        found.push(report(violation));
                    }
                }
                Statement::AlterFunction(alter) => {
                    if let Some(violation) = alter_function_violation(file, alter, &mut functions) {
                        found.push(report(violation));
                    }
                }
                Statement::Drop { names, .. } => {
                    for name in names {
                        tables.remove(&key(name));
                        views.remove(&key(name));
                        functions.remove(&key(name));
                    }
                }
                _ => {}
            }
        }
    }
    found.extend(unfinished(
        &tables,
        views.values().chain(functions.values()),
    ));
    found
}

fn unfinished<'a>(
    tables: &BTreeMap<String, Table>,
    gates: impl Iterator<Item = &'a View>,
) -> Vec<String> {
    let mut found = Vec::new();
    for table in tables.values() {
        if let Some(file) = &table.created_in
            && (!table.enabled || !table.forced)
        {
            found.push(format!(
                "{file}: CREATE TABLE {}: the same migration needs ALTER TABLE {} ENABLE ROW LEVEL SECURITY and ALTER TABLE {} FORCE ROW LEVEL SECURITY. {RULE}",
                table.label, table.label, table.label
            ));
        }
    }
    for table in tables.values() {
        if let (true, Some(("shared_read", file))) =
            (table.classes.contains_key(GATE), table.classes.get(API))
        {
            found.push(format!(
                "{file}: CREATE POLICY shared_read ON {}: {} has a view_read policy, so {API} reads only its own rows directly, with org_scope or parent_scope, and the rest through its gate views. {RULE}",
                table.label, table.label
            ));
        }
    }
    for gate in gates {
        if let View::Gate {
            file,
            label,
            revoked: false,
            ..
        } = gate
        {
            found.push(format!(
                "{file}: CREATE FUNCTION {label}: every role may EXECUTE a new function, so the same migration needs REVOKE EXECUTE ON FUNCTION {label} FROM PUBLIC and a GRANT EXECUTE to {API}, both before the OWNER TO. {RULE}"
            ));
        }
        if let View::Gate {
            kind,
            file,
            label,
            owned: false,
            ..
        } = gate
        {
            found.push(format!(
                "{file}: CREATE {kind} {label}: a gate {} runs as {GATE}, and the same migration needs ALTER {kind} {label} OWNER TO {GATE}. {RULE}",
                kind.to_lowercase()
            ));
        }
    }
    found
}

fn key(name: &ObjectName) -> String {
    parts(name).map_or_else(|| name.to_string(), |parts| parts.join("."))
}

fn alter_violation(
    file: &str,
    name: &ObjectName,
    operation: &AlterTableOperation,
    tables: &mut BTreeMap<String, Table>,
) -> Option<String> {
    let at = format!("ALTER TABLE {name} {operation}");
    if matches!(
        operation,
        AlterTableOperation::DisableRowLevelSecurity | AlterTableOperation::NoForceRowLevelSecurity
    ) {
        return Some(format!("{at}: row level security is never switched off."));
    }
    let table = tables
        .get_mut(&key(name))
        .filter(|table| table.created_in.as_deref() == Some(file))?;
    match operation {
        AlterTableOperation::EnableRowLevelSecurity => table.enabled = true,
        AlterTableOperation::ForceRowLevelSecurity => table.forced = true,
        _ => {}
    }
    None
}

fn policy_violation(
    file: &str,
    policy: &CreatePolicy,
    tables: &mut BTreeMap<String, Table>,
) -> Option<String> {
    let at = format!("CREATE POLICY {} ON {}", policy.name, policy.table_name);
    let Some(class) = CLASSES.iter().find(|class| class.name == policy.name.value) else {
        let names: Vec<&str> = CLASSES.iter().map(|class| class.name).collect();
        return Some(format!(
            "{at}: a policy is named after its class, one of {}.",
            names.join(", ")
        ));
    };
    let Some(table) = tables.get_mut(&key(&policy.table_name)) else {
        return Some(format!(
            "{at}: the migrations never create {}, so a policy on it cannot be checked.",
            policy.table_name
        ));
    };
    if table
        .classes
        .insert(class.group, (class.name, file.to_owned()))
        .is_some()
    {
        return Some(format!(
            "{at}: {} already has a policy for {}. Permissive policies add up, so a second one widens what the first allows.",
            policy.table_name, class.group
        ));
    }
    if matches!(policy.policy_type, Some(CreatePolicyType::Restrictive)) {
        return Some(format!(
            "{at}: a policy is PERMISSIVE. A RESTRICTIVE policy alone lets no row through."
        ));
    }
    let expected = expected_check(class, policy.using.as_ref());
    let command_matches = if class.select_only {
        matches!(policy.command, Some(CreatePolicyCommand::Select))
    } else {
        matches!(policy.command, None | Some(CreatePolicyCommand::All))
    };
    let to_matches =
        matches!(policy.to.as_deref(), Some([Owner::Ident(role)]) if role.value == class.group);
    let using_matches = policy
        .using
        .as_ref()
        .is_some_and(|using| same(using, &expected));
    let with_check_matches = match (&policy.with_check, class.select_only) {
        (None, true) => true,
        (Some(with_check), false) => same(with_check, &expected),
        _ => false,
    };
    if command_matches && to_matches && using_matches && with_check_matches {
        return None;
    }
    let command = if class.select_only {
        "FOR SELECT"
    } else {
        "FOR ALL"
    };
    let with_check = if class.select_only {
        String::new()
    } else {
        format!(" WITH CHECK ({expected})")
    };
    Some(format!(
        "{at}: a {} policy reads exactly CREATE POLICY {} ON {} {command} TO {} USING ({expected}){with_check}.",
        class.name, class.name, policy.table_name, class.group
    ))
}

fn expected_check(class: &Class, using: Option<&Expr>) -> String {
    match class.check {
        Check::Everything => "true".to_owned(),
        Check::Parent => using.and_then(parent_check).unwrap_or_else(|| {
            format!("EXISTS (SELECT 1 FROM <parent> p WHERE p.id = <column> AND p.<owner> = {ORGANIZATION})")
        }),
        Check::Organization => format!("organization_id = {ORGANIZATION}"),
    }
}

fn parent_check(using: &Expr) -> Option<String> {
    let Expr::Exists {
        subquery,
        negated: false,
    } = unnested(using)
    else {
        return None;
    };
    let SetExpr::Select(select) = subquery.body.as_ref() else {
        return None;
    };
    let [from] = select.from.as_slice() else {
        return None;
    };
    let TableFactor::Table {
        name,
        alias: Some(alias),
        ..
    } = &from.relation
    else {
        return None;
    };
    let alias = &alias.name;
    let Expr::BinaryOp {
        left: key,
        op: BinaryOperator::And,
        right: owner,
    } = unnested(select.selection.as_ref()?)
    else {
        return None;
    };
    let column = match unnested(key) {
        Expr::BinaryOp {
            op: BinaryOperator::Eq,
            right,
            ..
        } => match unnested(right) {
            Expr::Identifier(column) => column,
            _ => return None,
        },
        _ => return None,
    };
    let owner = match unnested(owner) {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            ..
        } => match unnested(left) {
            Expr::CompoundIdentifier(parts) => parts.last()?,
            _ => return None,
        },
        _ => return None,
    };
    Some(format!(
        "EXISTS (SELECT 1 FROM {name} {alias} WHERE {alias}.id = {column} AND {alias}.{owner} = {ORGANIZATION})"
    ))
}

fn same(expression: &Expr, expected: &str) -> bool {
    Parser::new(&PostgreSqlDialect {})
        .try_with_sql(expected)
        .and_then(|mut parser| parser.parse_expr())
        .is_ok_and(|expected| unnested(expression).to_string() == unnested(&expected).to_string())
}

fn unnested(expression: &Expr) -> &Expr {
    match expression {
        Expr::Nested(inner) => unnested(inner),
        _ => expression,
    }
}

fn view_kind(file: &str, view: &CreateView) -> Result<View, String> {
    if view.materialized {
        return Err(format!(
            "CREATE MATERIALIZED VIEW {}: a materialized view has no row level security, so every reader sees every row.",
            view.name
        ));
    }
    let options = match &view.options {
        CreateTableOptions::With(options) => options.as_slice(),
        _ => &[],
    };
    if invoker(options) == Some(true) {
        return Ok(View::Invoker);
    }
    if organization_filtered(&view.query) {
        Ok(View::Gate {
            kind: "VIEW",
            file: file.to_owned(),
            label: view.name.to_string(),
            owned: false,
            revoked: true,
        })
    } else {
        Err(format!(
            "CREATE VIEW {}: write CREATE VIEW {} WITH (security_invoker = true) AS ..., so the view reads with the caller's row level security, or make it a gate view whose WHERE has <column> = {ORGANIZATION} or (<column> IS NULL OR <column> = {ORGANIZATION}) as a condition joined by AND.",
            view.name, view.name
        ))
    }
}

fn organization_filtered(query: &Query) -> bool {
    match query.body.as_ref() {
        SetExpr::Select(select) => select
            .selection
            .as_ref()
            .is_some_and(|selection| conjuncts(selection).into_iter().any(organization_filter)),
        _ => false,
    }
}

fn conjuncts(expression: &Expr) -> Vec<&Expr> {
    match unnested(expression) {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        other => vec![other],
    }
}

fn organization_filter(expression: &Expr) -> bool {
    let Some(column) = filtered_column(expression) else {
        return false;
    };
    let exact = format!("{column} = {ORGANIZATION}");
    let shared = format!("{column} IS NULL OR {column} = {ORGANIZATION}");
    same(expression, &exact) || same(expression, &shared)
}

fn filtered_column(expression: &Expr) -> Option<String> {
    let column = match unnested(expression) {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Or,
            ..
        } => match unnested(left) {
            Expr::IsNull(column) => column.as_ref(),
            _ => return None,
        },
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            ..
        } => left.as_ref(),
        _ => return None,
    };
    match unnested(column) {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) => Some(column.to_string()),
        _ => None,
    }
}

fn owned_gate(gate: View, revoked: bool) -> View {
    match gate {
        View::Gate {
            kind, file, label, ..
        } => View::Gate {
            kind,
            file,
            label,
            owned: true,
            revoked,
        },
        View::Invoker => View::Invoker,
    }
}

fn define_function(
    file: &str,
    function: &CreateFunction,
    functions: &mut BTreeMap<String, View>,
) -> Option<String> {
    let gate = match gate_function(file, function) {
        Ok(gate) => gate,
        Err(violation) => return Some(violation),
    };
    let previous = match functions.get(&key(&function.name)) {
        Some(View::Gate {
            owned: true,
            revoked,
            ..
        }) if function.or_replace => Some(*revoked),
        _ => None,
    };
    functions.insert(
        key(&function.name),
        match previous {
            Some(revoked) => owned_gate(gate, revoked),
            None => gate,
        },
    );
    None
}

fn gate_function(file: &str, function: &CreateFunction) -> Result<View, String> {
    let at = format!("CREATE FUNCTION {}", function.name);
    let sql = function
        .language
        .as_ref()
        .is_some_and(|language| language.value.eq_ignore_ascii_case("sql"));
    let search_path = function.set_params.iter().any(|param| {
        parts(&param.name).is_some_and(|parts| parts == ["search_path"])
            && matches!(&param.value, FunctionSetValue::Values(values) if values.iter().map(ToString::to_string).collect::<Vec<_>>() == ["pg_catalog", "pg_temp"])
    });
    let filtered = function_query(function).is_some_and(|query| organization_filtered(&query));
    if sql && search_path && filtered {
        Ok(View::Gate {
            kind: "FUNCTION",
            file: file.to_owned(),
            label: function.name.to_string(),
            owned: false,
            revoked: false,
        })
    } else {
        Err(format!(
            "{at}: a SECURITY DEFINER function runs as its owner and skips row level security, so it is only allowed as a gate function: LANGUAGE sql, SET search_path = pg_catalog, pg_temp, a body that is one SELECT whose WHERE has <column> = {ORGANIZATION} or (<column> IS NULL OR <column> = {ORGANIZATION}) joined by AND, and ALTER FUNCTION ... OWNER TO {GATE}. Otherwise leave it SECURITY INVOKER."
        ))
    }
}

fn privilege_violation(
    command: &str,
    privileges: &Privileges,
    objects: Option<&GrantObjects>,
    grantees: &[Grantee],
    functions: &mut BTreeMap<String, View>,
) -> Option<String> {
    let Some(GrantObjects::Functions(targets)) = objects else {
        return None;
    };
    let execute = match privileges {
        Privileges::All { .. } => true,
        Privileges::Actions(actions) => actions
            .iter()
            .any(|action| matches!(action, Action::Execute { .. })),
    };
    let public = grantees.iter().any(|grantee| {
        grantee.grantee_type == GranteesType::Public
            || matches!(&grantee.name, Some(GranteeName::ObjectName(name)) if parts(name).is_some_and(|parts| parts == ["public"]))
    });
    for target in targets {
        let Some(View::Gate { owned, revoked, .. }) = functions.get_mut(&key(&target.name)) else {
            continue;
        };
        if *owned {
            return Some(format!(
                "{command} ON FUNCTION {}: after OWNER TO {GATE} the migration role no longer owns the function, so the privilege change has no effect. Put it before the OWNER TO.",
                target.name
            ));
        }
        match (command, execute && public) {
            ("GRANT", true) => {
                return Some(format!(
                    "GRANT ON FUNCTION {}: a gate function skips row level security, so EXECUTE goes to {API} only, never to PUBLIC.",
                    target.name
                ));
            }
            ("REVOKE", true) => *revoked = true,
            _ => {}
        }
    }
    None
}

fn function_query(function: &CreateFunction) -> Option<Query> {
    let (CreateFunctionBody::AsBeforeOptions { body, .. }
    | CreateFunctionBody::AsAfterOptions(body)) = function.function_body.as_ref()?
    else {
        return None;
    };
    let Expr::Value(value) = body else {
        return None;
    };
    let text = match &value.value {
        Value::SingleQuotedString(text) => text.clone(),
        Value::DollarQuotedString(text) => text.value.clone(),
        _ => return None,
    };
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, &text).ok()?;
    match statements.as_slice() {
        [Statement::Query(query)] => Some(query.as_ref().clone()),
        _ => None,
    }
}

fn alter_function_violation(
    file: &str,
    alter: &AlterFunction,
    functions: &mut BTreeMap<String, View>,
) -> Option<String> {
    let name = &alter.function.name;
    let gate = functions.get_mut(&key(name));
    match (&alter.operation, gate) {
        (
            AlterFunctionOperation::OwnerTo(owner),
            Some(View::Gate {
                file: created,
                owned,
                ..
            }),
        ) => {
            if matches!(owner, Owner::Ident(role) if role.value == GATE) {
                *owned |= created == file;
                None
            } else {
                Some(format!(
                    "ALTER FUNCTION {name} OWNER TO {owner}: a gate function belongs to {GATE}."
                ))
            }
        }
        (AlterFunctionOperation::Actions { actions, .. }, gate) => {
            let definer = actions.iter().any(|action| {
                matches!(
                    action,
                    AlterFunctionAction::Security {
                        security: FunctionSecurity::Definer,
                        ..
                    }
                )
            });
            let search_path = actions.iter().any(|action| match action {
                AlterFunctionAction::Set(param) => {
                    parts(&param.name).is_some_and(|parts| parts == ["search_path"])
                }
                AlterFunctionAction::Reset(_) => true,
                _ => false,
            });
            if definer && gate.is_none() {
                Some(format!(
                    "ALTER FUNCTION {name} SECURITY DEFINER: only a gate function is SECURITY DEFINER, and it is created as one."
                ))
            } else if search_path && gate.is_some() {
                Some(format!(
                    "ALTER FUNCTION {name}: a gate function keeps SET search_path = pg_catalog, pg_temp."
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn alter_view_violation(
    file: &str,
    name: &ObjectName,
    operation: &AlterViewOperation,
    views: &mut BTreeMap<String, View>,
) -> Option<String> {
    let view = views.get_mut(&key(name));
    match (operation, view) {
        (
            AlterViewOperation::OwnerTo(owner),
            Some(View::Gate {
                file: created,
                owned,
                ..
            }),
        ) => {
            if matches!(owner, Owner::Ident(role) if role.value == GATE) {
                *owned |= created == file;
                None
            } else {
                Some(format!(
                    "ALTER VIEW {name} OWNER TO {owner}: a gate view belongs to {GATE}."
                ))
            }
        }
        (
            AlterViewOperation::As {
                query,
                with_options,
                ..
            },
            Some(view),
        ) => {
            if invoker(with_options) == Some(true) {
                *view = View::Invoker;
                None
            } else if matches!(view, View::Gate { .. }) && organization_filtered(query) {
                None
            } else {
                Some(format!(
                    "ALTER VIEW {name} AS: a view stays a security_invoker view, or a gate view with its organization filter."
                ))
            }
        }
        (AlterViewOperation::SetOptions { options }, Some(view)) => {
            if matches!(view, View::Invoker) && invoker(options) == Some(false) {
                Some(format!(
                    "ALTER VIEW {name}: a view stays a security_invoker view."
                ))
            } else {
                if invoker(options) == Some(true) {
                    *view = View::Invoker;
                }
                None
            }
        }
        (AlterViewOperation::ResetOptions { options }, Some(View::Invoker)) => options
            .iter()
            .any(|option| option.value.eq_ignore_ascii_case("security_invoker"))
            .then(|| format!("ALTER VIEW {name}: a view stays a security_invoker view.")),
        _ => None,
    }
}

fn invoker(options: &[SqlOption]) -> Option<bool> {
    options.iter().find_map(|option| match option {
        SqlOption::Ident(name) if is_invoker(name) => Some(true),
        SqlOption::KeyValue { key, value } if is_invoker(key) => Some(matches!(
            value.to_string().to_lowercase().trim_matches('\''),
            "true" | "on" | "1" | "yes"
        )),
        _ => None,
    })
}

fn is_invoker(name: &ObjectName) -> bool {
    parts(name).is_some_and(|parts| parts == ["security_invoker"])
}

#[cfg(test)]
mod tests {
    const FILE: &str = "crates/db/migrations/0001_test.sql";
    const LATER: &str = "crates/db/migrations/0002_test.sql";

    fn found(sql: &str) -> Vec<String> {
        super::violations(&[(FILE.to_owned(), sql.to_owned())])
    }

    fn found_in_two(first: &str, second: &str) -> Vec<String> {
        super::violations(&[
            (FILE.to_owned(), first.to_owned()),
            (LATER.to_owned(), second.to_owned()),
        ])
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

    const TABLE: &str = "
        CREATE TABLE catalog.item (id uuid PRIMARY KEY, organization_id text);
        ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY;
        ALTER TABLE catalog.item FORCE ROW LEVEL SECURITY;
    ";

    const CHILD: &str = "
        CREATE TABLE catalog.item_tag (id uuid PRIMARY KEY, item_id uuid NOT NULL, tag text NOT NULL);
        ALTER TABLE catalog.item_tag ENABLE ROW LEVEL SECURITY;
        ALTER TABLE catalog.item_tag FORCE ROW LEVEL SECURITY;
    ";

    const ORG_SCOPE: &str = "CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'));";

    #[test]
    fn every_policy_class_passes_in_its_exact_form() {
        for policy in [
            ORG_SCOPE,
            "CREATE POLICY org_scope ON catalog.item FOR ALL TO app_api USING ((organization_id = current_setting('app.organization_id'))) WITH CHECK (organization_id   =   current_setting('app.organization_id'))",
            "CREATE POLICY admin_access ON catalog.item TO app_admin USING (true) WITH CHECK (true)",
            "CREATE POLICY shared_read ON catalog.item FOR SELECT TO app_api USING (true)",
            "CREATE POLICY view_read ON catalog.item FOR SELECT TO app_view USING (true)",
        ] {
            assert_eq!(
                found(&format!("{TABLE} {policy};")),
                Vec::<String>::new(),
                "{policy}"
            );
        }
    }

    #[test]
    fn a_table_has_one_policy_per_group() {
        let sql = format!(
            "{TABLE} {ORG_SCOPE} CREATE POLICY admin_access ON catalog.item TO app_admin USING (true) WITH CHECK (true);"
        );
        assert_eq!(found(&sql), Vec::<String>::new());
        let found = found_in_two(
            &format!(
                "{TABLE} CREATE POLICY shared_read ON catalog.item FOR SELECT TO app_api USING (true);"
            ),
            ORG_SCOPE,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with(&format!(
                "{LATER}: CREATE POLICY org_scope ON catalog.item: catalog.item already has a policy for app_api."
            )),
            "{found:?}"
        );
    }

    #[test]
    fn a_table_without_a_policy_passes() {
        assert_eq!(found(TABLE), Vec::<String>::new());
    }

    #[test]
    fn a_table_needs_enabled_and_forced_row_level_security_in_its_own_migration() {
        let expected = "CREATE TABLE catalog.item: the same migration needs ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY and ALTER TABLE catalog.item FORCE ROW LEVEL SECURITY.";
        let create = "CREATE TABLE catalog.item (id uuid);";
        assert_one(create, expected);
        assert_one(
            &format!("{create} ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY;"),
            expected,
        );
        assert_one(
            &format!("{create} ALTER TABLE catalog.item FORCE ROW LEVEL SECURITY;"),
            expected,
        );
        let later = found_in_two(
            create,
            "ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY; ALTER TABLE catalog.item FORCE ROW LEVEL SECURITY;",
        );
        assert_eq!(later.len(), 1, "{later:?}");
        assert!(
            later[0].starts_with(&format!("{FILE}: {expected}")),
            "{later:?}"
        );
    }

    #[test]
    fn row_level_security_is_never_switched_off() {
        for operation in ["DISABLE", "NO FORCE"] {
            assert_one(
                &format!("{TABLE} ALTER TABLE catalog.item {operation} ROW LEVEL SECURITY;"),
                &format!(
                    "ALTER TABLE catalog.item {operation} ROW LEVEL SECURITY: row level security is never switched off."
                ),
            );
        }
    }

    #[test]
    fn org_scope_reads_organization_id() {
        assert_one(
            &format!(
                "{TABLE} CREATE POLICY org_scope ON catalog.item TO app_api USING (owner_id = current_setting('app.organization_id')) WITH CHECK (owner_id = current_setting('app.organization_id'));"
            ),
            "CREATE POLICY org_scope ON catalog.item: a org_scope policy reads exactly CREATE POLICY org_scope ON catalog.item FOR ALL TO app_api USING (organization_id = ",
        );
    }

    #[test]
    fn a_policy_that_differs_from_its_class_is_reported() {
        for policy in [
            "CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = organization_id) WITH CHECK (organization_id = organization_id)",
            "CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = current_setting('app.organization_id') OR true) WITH CHECK (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = current_setting('app.organization_id', true)) WITH CHECK (organization_id = current_setting('app.organization_id', true))",
            "CREATE POLICY org_scope ON catalog.item FOR SELECT TO app_api USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY org_scope ON catalog.item USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY org_scope ON catalog.item TO app_admin USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY org_scope ON catalog.item TO app_api, app_admin USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY org_scope ON catalog.item TO PUBLIC USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'))",
            "CREATE POLICY admin_access ON catalog.item TO app_api USING (true) WITH CHECK (true)",
            "CREATE POLICY admin_access ON catalog.item TO app_admin USING (true)",
            "CREATE POLICY shared_read ON catalog.item TO app_api USING (true)",
            "CREATE POLICY shared_read ON catalog.item FOR SELECT TO app_api USING (true) WITH CHECK (true)",
            "CREATE POLICY shared_read ON catalog.item FOR SELECT TO CURRENT_USER USING (true)",
            "CREATE POLICY view_read ON catalog.item FOR SELECT TO app_api USING (true)",
        ] {
            let found = found(&format!("{TABLE} {policy};"));
            assert_eq!(found.len(), 1, "{policy}: {found:?}");
            assert!(
                found[0].contains(" policy reads exactly CREATE POLICY "),
                "{policy}: {found:?}"
            );
        }
    }

    #[test]
    fn a_policy_has_a_known_class_name() {
        assert_one(
            &format!("{TABLE} CREATE POLICY everyone ON catalog.item USING (true);"),
            "CREATE POLICY everyone ON catalog.item: a policy is named after its class, one of org_scope, parent_scope, shared_read, admin_access, view_read.",
        );
    }

    #[test]
    fn a_policy_is_permissive() {
        assert_one(
            &format!(
                "{TABLE} CREATE POLICY shared_read ON catalog.item AS RESTRICTIVE FOR SELECT TO app_api USING (true);"
            ),
            "CREATE POLICY shared_read ON catalog.item: a policy is PERMISSIVE.",
        );
    }

    #[test]
    fn a_policy_on_a_table_the_migrations_never_create_is_reported() {
        assert_one(
            "CREATE POLICY shared_read ON catalog.elsewhere FOR SELECT TO app_api USING (true);",
            "CREATE POLICY shared_read ON catalog.elsewhere: the migrations never create catalog.elsewhere",
        );
    }

    #[test]
    fn a_dropped_table_can_be_created_again() {
        let policy =
            "CREATE POLICY shared_read ON catalog.item FOR SELECT TO app_api USING (true);";
        let sql = format!("{TABLE} {policy} DROP TABLE catalog.item; {TABLE} {policy}");
        assert_eq!(found(&sql), Vec::<String>::new());
    }

    #[test]
    fn a_view_is_a_security_invoker_view() {
        for view in [
            "CREATE VIEW catalog.titles WITH (security_invoker = true) AS SELECT id FROM catalog.item",
            "CREATE VIEW catalog.titles WITH (security_invoker) AS SELECT id FROM catalog.item",
        ] {
            assert_eq!(found(view), Vec::<String>::new(), "{view}");
        }
        for view in [
            "CREATE VIEW catalog.titles AS SELECT id FROM catalog.item",
            "CREATE VIEW catalog.titles WITH (security_invoker = false) AS SELECT id FROM catalog.item",
            "CREATE VIEW catalog.titles WITH (security_barrier) AS SELECT id FROM catalog.item",
        ] {
            assert_one(
                view,
                "CREATE VIEW catalog.titles: write CREATE VIEW catalog.titles WITH (security_invoker = true) AS",
            );
        }
    }

    #[test]
    fn a_materialized_view_is_reported() {
        assert_one(
            "CREATE MATERIALIZED VIEW catalog.titles AS SELECT id FROM catalog.item",
            "CREATE MATERIALIZED VIEW catalog.titles: a materialized view has no row level security",
        );
    }

    #[test]
    fn a_view_stays_a_security_invoker_view() {
        let view = "CREATE VIEW catalog.titles WITH (security_invoker = true) AS SELECT id FROM catalog.item;";
        for alter in [
            "ALTER VIEW catalog.titles SET (security_invoker = false)",
            "ALTER VIEW catalog.titles RESET (security_invoker)",
        ] {
            assert_one(
                &format!("{view} {alter}"),
                "ALTER VIEW catalog.titles: a view stays a security_invoker view.",
            );
        }
        assert_one(
            &format!("{view} ALTER VIEW catalog.titles AS SELECT id FROM catalog.item"),
            "ALTER VIEW catalog.titles AS: a view stays a security_invoker view",
        );
        assert_eq!(
            found(&format!(
                "{view} ALTER VIEW catalog.titles SET (security_invoker = true)"
            )),
            Vec::<String>::new()
        );
    }

    const GATED: &str = "
        CREATE TABLE catalog.item (id uuid PRIMARY KEY, organization_id text, title text NOT NULL);
        ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY;
        ALTER TABLE catalog.item FORCE ROW LEVEL SECURITY;
        CREATE POLICY view_read ON catalog.item FOR SELECT TO app_view USING (true);
        GRANT SELECT ON catalog.item TO app_view;
    ";

    const GATE_VIEW: &str = "
        CREATE VIEW catalog.visible_item AS
            SELECT i.id, i.organization_id, i.title FROM catalog.item i
            WHERE i.title <> '' AND (i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id'));
    ";

    const OWNED: &str = "GRANT SELECT ON catalog.visible_item TO app_api; ALTER VIEW catalog.visible_item OWNER TO app_view;";

    #[test]
    fn a_gate_view_passes() {
        assert_eq!(
            found(&format!("{GATED} {GATE_VIEW} {OWNED}")),
            Vec::<String>::new()
        );
        let exact = "CREATE VIEW catalog.own AS SELECT id FROM catalog.item WHERE organization_id = current_setting('app.organization_id'); ALTER VIEW catalog.own OWNER TO app_view;";
        assert_eq!(found(&format!("{TABLE} {exact}")), Vec::<String>::new());
    }

    #[test]
    fn a_gate_view_is_owned_by_the_gate_role_in_its_own_migration() {
        let expected = "CREATE VIEW catalog.visible_item: a gate view runs as app_view, and the same migration needs ALTER VIEW catalog.visible_item OWNER TO app_view.";
        assert_one(GATE_VIEW, expected);
        let later = found_in_two(GATE_VIEW, OWNED);
        assert_eq!(later.len(), 1, "{later:?}");
        assert!(
            later[0].starts_with(&format!("{FILE}: {expected}")),
            "{later:?}"
        );
        assert_one(
            &format!("{GATE_VIEW} {OWNED} ALTER VIEW catalog.visible_item OWNER TO app_api;"),
            "ALTER VIEW catalog.visible_item OWNER TO app_api: a gate view belongs to app_view.",
        );
    }

    #[test]
    fn a_gate_view_filters_by_organization() {
        for view in [
            "CREATE VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i",
            "CREATE VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i WHERE i.organization_id IS NULL",
            "CREATE VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i WHERE i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id') OR true",
            "CREATE VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i WHERE i.organization_id = current_setting('app.organization_id') OR true",
            "CREATE VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i WHERE a.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id')",
            "CREATE VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i WHERE i.organization_id = current_setting('app.other')",
            "CREATE VIEW catalog.visible_item AS SELECT id FROM catalog.a UNION SELECT id FROM catalog.b",
        ] {
            assert_one(
                &format!("{view}; {OWNED}"),
                "CREATE VIEW catalog.visible_item: write CREATE VIEW catalog.visible_item WITH (security_invoker = true) AS",
            );
        }
        assert_one(
            &format!(
                "{GATE_VIEW} {OWNED} ALTER VIEW catalog.visible_item AS SELECT i.id FROM catalog.item i;"
            ),
            "ALTER VIEW catalog.visible_item AS: a view stays a security_invoker view, or a gate view with its organization filter.",
        );
    }

    const PARENT: &str = "EXISTS (SELECT 1 FROM catalog.item i WHERE i.id = item_id AND i.organization_id = current_setting('app.organization_id'))";

    #[test]
    fn a_gated_table_is_written_with_its_own_rows_only() {
        let sql = format!(
            "{GATED} {CHILD}
            CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = current_setting('app.organization_id')) WITH CHECK (organization_id = current_setting('app.organization_id'));
            CREATE POLICY parent_scope ON catalog.item_tag TO app_api USING ({PARENT}) WITH CHECK ({PARENT});
            CREATE POLICY view_read ON catalog.item_tag FOR SELECT TO app_view USING (true);
            GRANT SELECT, INSERT, UPDATE, DELETE ON catalog.item, catalog.item_tag TO app_api;"
        );
        assert_eq!(found(&sql), Vec::<String>::new());
    }

    #[test]
    fn a_gated_table_has_no_shared_read_for_the_api() {
        let shared =
            "CREATE POLICY shared_read ON catalog.item FOR SELECT TO app_api USING (true);";
        assert_one(
            &format!("{GATED} {shared}"),
            "CREATE POLICY shared_read ON catalog.item: catalog.item has a view_read policy",
        );
        let split = found_in_two(
            &format!("{TABLE} {shared}"),
            "CREATE POLICY view_read ON catalog.item FOR SELECT TO app_view USING (true);",
        );
        assert_eq!(split.len(), 1, "{split:?}");
        assert!(
            split[0].starts_with(&format!(
                "{FILE}: CREATE POLICY shared_read ON catalog.item"
            )),
            "{split:?}"
        );
    }

    #[test]
    fn a_parent_scope_policy_reads_its_owner_through_the_parent() {
        assert_eq!(
            found(&format!(
                "{TABLE} {CHILD} CREATE POLICY parent_scope ON catalog.item_tag TO app_api USING ({PARENT}) WITH CHECK ({PARENT});"
            )),
            Vec::<String>::new()
        );
        for (using, with_check) in [
            (PARENT, "true"),
            (
                "EXISTS (SELECT 1 FROM catalog.item i WHERE i.id = item_id AND (i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id')))",
                PARENT,
            ),
            (
                "EXISTS (SELECT 1 FROM catalog.item i WHERE i.id = item_id AND i.organization_id = current_setting('app.organization_id') OR true)",
                PARENT,
            ),
            (
                "NOT EXISTS (SELECT 1 FROM catalog.item i WHERE i.id = item_id AND i.organization_id = current_setting('app.organization_id'))",
                PARENT,
            ),
            (
                "EXISTS (SELECT 1 FROM catalog.item i WHERE i.id = item_id AND i.organization_id = current_setting('app.other'))",
                PARENT,
            ),
            (
                "EXISTS (SELECT 1 FROM catalog.item i, catalog.other o WHERE i.id = item_id AND i.organization_id = current_setting('app.organization_id'))",
                PARENT,
            ),
            ("true", "true"),
        ] {
            let found = found(&format!(
                "{TABLE} {CHILD} CREATE POLICY parent_scope ON catalog.item_tag TO app_api USING ({using}) WITH CHECK ({with_check});"
            ));
            assert_eq!(found.len(), 1, "{using}: {found:?}");
            assert!(
                found[0].contains("a parent_scope policy reads exactly"),
                "{using}: {found:?}"
            );
        }
    }

    #[test]
    fn a_security_definer_function_is_reported() {
        let body = "RETURNS integer LANGUAGE sql AS $$ SELECT 1 $$";
        assert_one(
            &format!("CREATE FUNCTION catalog.one() {body} SECURITY DEFINER"),
            "CREATE FUNCTION catalog.one: a SECURITY DEFINER function runs as its owner",
        );
        for function in [
            format!("CREATE FUNCTION catalog.one() {body}"),
            format!("CREATE FUNCTION catalog.one() {body} SECURITY INVOKER"),
        ] {
            assert_eq!(found(&function), Vec::<String>::new(), "{function}");
        }
    }

    const GATE_FUNCTION: &str = concat!(
        "
        CREATE FUNCTION catalog.search_items(term text) RETURNS TABLE (id uuid, organization_id text, title text)
            LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp
            AS $$
                SELECT i.id, i.organization_id, i.title FROM catalog.item i
                WHERE i.search @@ websearch_to_tsquery('simple', term)
                    AND (i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id'))
            $$;
    ",
        "REVOKE ALL ON FUNCTION catalog.search_items(text) FROM PUBLIC;",
        "GRANT EXECUTE ON FUNCTION catalog.search_items(text) TO app_api;"
    );

    const FUNCTION_OWNED: &str = "ALTER FUNCTION catalog.search_items(text) OWNER TO app_view;";

    fn created_only() -> &'static str {
        GATE_FUNCTION
            .split_once("REVOKE")
            .map_or("", |(created, _)| created)
    }

    #[test]
    fn a_gate_function_passes() {
        assert_eq!(
            found(&format!("{GATE_FUNCTION} {FUNCTION_OWNED}")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_gate_function_is_owned_by_the_gate_role_in_its_own_migration() {
        let expected = "CREATE FUNCTION catalog.search_items: a gate function runs as app_view, and the same migration needs ALTER FUNCTION catalog.search_items OWNER TO app_view.";
        assert_one(GATE_FUNCTION, expected);
        let later = found_in_two(GATE_FUNCTION, FUNCTION_OWNED);
        assert_eq!(later.len(), 1, "{later:?}");
        assert!(
            later[0].starts_with(&format!("{FILE}: {expected}")),
            "{later:?}"
        );
        assert_one(
            &format!(
                "{GATE_FUNCTION} {FUNCTION_OWNED} ALTER FUNCTION catalog.search_items(text) OWNER TO app_api;"
            ),
            "ALTER FUNCTION catalog.search_items OWNER TO app_api: a gate function belongs to app_view.",
        );
    }

    #[test]
    fn a_gate_function_is_closed_to_public_before_its_owner_changes() {
        let created = created_only();
        assert_one(
            &format!("{created} {FUNCTION_OWNED}"),
            "CREATE FUNCTION catalog.search_items: every role may EXECUTE a new function, so the same migration needs REVOKE EXECUTE ON FUNCTION catalog.search_items FROM PUBLIC",
        );
        assert_eq!(
            found(&format!(
                "{created} REVOKE EXECUTE ON FUNCTION catalog.search_items(text) FROM public; {FUNCTION_OWNED}"
            )),
            Vec::<String>::new()
        );
        let late = found(&format!(
            "{created} {FUNCTION_OWNED} REVOKE ALL ON FUNCTION catalog.search_items(text) FROM PUBLIC;"
        ));
        assert_eq!(late.len(), 2, "{late:?}");
        assert!(
            late[0].contains("REVOKE ON FUNCTION catalog.search_items: after OWNER TO app_view the migration role no longer owns the function"),
            "{late:?}"
        );
        assert!(late[1].contains("needs REVOKE EXECUTE"), "{late:?}");
        assert_one(
            &format!(
                "{GATE_FUNCTION} {FUNCTION_OWNED} GRANT EXECUTE ON FUNCTION catalog.search_items(text) TO app_api;"
            ),
            "GRANT ON FUNCTION catalog.search_items: after OWNER TO app_view",
        );
        assert_one(
            &format!(
                "{GATE_FUNCTION} GRANT EXECUTE ON FUNCTION catalog.search_items(text) TO PUBLIC; {FUNCTION_OWNED}"
            ),
            "GRANT ON FUNCTION catalog.search_items: a gate function skips row level security, so EXECUTE goes to app_api only, never to PUBLIC.",
        );
        let replaced = GATE_FUNCTION.replace("CREATE FUNCTION", "CREATE OR REPLACE FUNCTION");
        let replaced = replaced
            .split_once("REVOKE")
            .map_or("", |(created, _)| created);
        assert_eq!(
            found(&format!("{GATE_FUNCTION} {FUNCTION_OWNED} {replaced}")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_security_definer_function_must_be_a_gate_function() {
        let filter = "WHERE i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id')";
        for function in [
            format!(
                "CREATE FUNCTION catalog.f() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER AS $$ SELECT i.id FROM catalog.item i {filter} $$"
            ),
            format!(
                "CREATE FUNCTION catalog.f() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER SET search_path = public AS $$ SELECT i.id FROM catalog.item i {filter} $$"
            ),
            format!(
                "CREATE FUNCTION catalog.f() RETURNS SETOF uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$ BEGIN RETURN QUERY SELECT i.id FROM catalog.item i {filter}; END $$"
            ),
            "CREATE FUNCTION catalog.f() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$ SELECT i.id FROM catalog.item i $$".to_owned(),
            "CREATE FUNCTION catalog.f() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$ SELECT i.id FROM catalog.item i WHERE i.organization_id = current_setting('app.organization_id') OR true $$".to_owned(),
            format!(
                "CREATE FUNCTION catalog.f() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$ SELECT i.id FROM catalog.item i {filter}; DELETE FROM catalog.item $$"
            ),
        ] {
            assert_one(
                &format!("{function}; ALTER FUNCTION catalog.f() OWNER TO app_view;"),
                "CREATE FUNCTION catalog.f: a SECURITY DEFINER function runs as its owner and skips row level security, so it is only allowed as a gate function",
            );
        }
    }

    #[test]
    fn a_function_is_not_made_security_definer_later() {
        assert_one(
            "CREATE FUNCTION catalog.f() RETURNS integer LANGUAGE sql AS $$ SELECT 1 $$; ALTER FUNCTION catalog.f() SECURITY DEFINER;",
            "ALTER FUNCTION catalog.f SECURITY DEFINER: only a gate function is SECURITY DEFINER",
        );
        assert_one(
            &format!(
                "{GATE_FUNCTION} {FUNCTION_OWNED} ALTER FUNCTION catalog.search_items(text) RESET search_path;"
            ),
            "ALTER FUNCTION catalog.search_items: a gate function keeps SET search_path = pg_catalog, pg_temp.",
        );
    }
}
