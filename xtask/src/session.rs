use std::fs;
use std::path::Path;

use crate::migrations::sorted_entries;

pub(crate) const SCOPE: &str = "crates/db/queries/session/scope.sql";
const RULE: &str = "Row level security reads the request's organization and user from the app.* settings, so they are set in exactly one place: the transaction-local set_config calls in crates/db/queries/session/scope.sql, which the app crate runs through db::scope at the start of every transaction. Nothing else calls set_config or runs SET app.* , not Rust code, not a query, not a migration, not ALTER ROLE or ALTER DATABASE.";

pub(crate) fn sources(workspace: &Path) -> Result<Vec<(String, String)>, String> {
    let mut found = Vec::new();
    for top in ["crates", "bins"] {
        collect(workspace, &workspace.join(top), &mut found)?;
    }
    Ok(found)
}

fn collect(
    workspace: &Path,
    directory: &Path,
    found: &mut Vec<(String, String)>,
) -> Result<(), String> {
    if !directory.is_dir() {
        return Ok(());
    }
    for path in sorted_entries(directory)? {
        if path.is_dir() {
            collect(workspace, &path, found)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "rs" || extension == "sql")
        {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("{}: cannot read: {error}", path.display()))?;
            let file = path
                .strip_prefix(workspace)
                .unwrap_or(&path)
                .display()
                .to_string();
            found.push((file, text));
        }
    }
    Ok(())
}

pub(crate) fn violations(files: &[(String, String)]) -> Vec<String> {
    files
        .iter()
        .filter(|(file, text)| file != SCOPE && sets_a_setting(text))
        .map(|(file, _)| format!("{file}: sets an app.* setting. {RULE}"))
        .collect()
}

fn sets_a_setting(text: &str) -> bool {
    let text = text.to_lowercase();
    let words: Vec<&str> = text
        .split(|character: char| {
            !(character.is_alphanumeric() || character == '_' || character == '.')
        })
        .filter(|word| !word.is_empty())
        .collect();
    words.iter().enumerate().any(|(index, word)| {
        *word == "set_config"
            || (*word == "set"
                && words[index + 1..]
                    .iter()
                    .find(|next| !matches!(**next, "local" | "session"))
                    .is_some_and(|next| next.starts_with("app.")))
    })
}

#[cfg(test)]
mod tests {
    use super::{SCOPE, violations};

    fn found(file: &str, text: &str) -> Vec<String> {
        violations(&[(file.to_owned(), text.to_owned())])
    }

    #[test]
    fn the_scope_query_sets_the_settings() {
        let scope = "SELECT set_config('app.organization_id', $1, true), set_config('app.user_id', $2, true)";
        assert_eq!(found(SCOPE, scope), Vec::<String>::new());
    }

    #[test]
    fn every_other_place_that_sets_a_setting_is_reported() {
        for (file, text) in [
            (
                "crates/db/queries/catalog/list_items.sql",
                "SELECT set_config('app.organization_id', '1', false)",
            ),
            (
                "crates/db/src/catalog.rs",
                r#"conn.execute("SET app.organization_id = 1").await?;"#,
            ),
            (
                "crates/db/src/catalog.rs",
                r#"conn.execute("set local app.user_id = 'user_1'")"#,
            ),
            (
                "crates/db/migrations/0002_catalog.sql",
                "ALTER ROLE app_api SET app.organization_id = '1';",
            ),
            (
                "crates/db/migrations/0002_catalog.sql",
                "ALTER DATABASE app SET\n  app.organization_id TO '1';",
            ),
            (
                "bins/migrate/src/main.rs",
                r#"let sql = "SET SESSION app.organization_id = 1";"#,
            ),
            (
                "crates/db/migrations/0002_catalog.sql",
                "CREATE FUNCTION research.f() RETURNS void LANGUAGE sql AS $$ SELECT SET_CONFIG('app.organization_id', '1', true) $$;",
            ),
        ] {
            let violations = found(file, text);
            assert_eq!(violations.len(), 1, "{text}");
            assert!(
                violations[0].starts_with(&format!("{file}: sets an app.* setting.")),
                "{violations:?}"
            );
        }
    }

    #[test]
    fn reading_a_setting_or_setting_something_else_passes() {
        for text in [
            "CREATE POLICY org_scope ON catalog.item TO app_api USING (organization_id = current_setting('app.organization_id')::integer);",
            "SET statement_timeout = '5s';",
            "SET LOCAL search_path = research;",
            "UPDATE catalog.item SET app_name = 'x' WHERE id = 1;",
            "fn offset(self) -> Self { self.set(1) }",
        ] {
            assert_eq!(
                found("crates/db/queries/catalog/list_items.sql", text),
                Vec::<String>::new(),
                "{text}"
            );
        }
    }
}
