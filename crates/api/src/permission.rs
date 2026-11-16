pub const EXTENSION: &str = "x-permission";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    SignedIn,
    Member,
}

impl Permission {
    pub const ALL: [Self; 2] = [Self::SignedIn, Self::Member];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SignedIn => "signed_in",
            Self::Member => "member",
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{EXTENSION, Permission};

    fn undeclared(spec: &Value) -> Vec<String> {
        let catalog: Vec<&str> = Permission::ALL
            .iter()
            .map(|permission| permission.as_str())
            .collect();
        let mut found = Vec::new();
        for (path, item) in spec["paths"].as_object().into_iter().flatten() {
            for (method, operation) in item.as_object().into_iter().flatten() {
                if ![
                    "get", "put", "post", "delete", "options", "head", "patch", "trace",
                ]
                .contains(&method.as_str())
                {
                    continue;
                }
                let declared = operation.get(EXTENSION).and_then(Value::as_str);
                if !declared.is_some_and(|permission| catalog.contains(&permission)) {
                    found.push(format!(
                        "{} {path}: declares {declared:?} as {EXTENSION}. Every route names one permission from crates/api/src/permission.rs: {}.",
                        method.to_uppercase(),
                        catalog.join(", ")
                    ));
                }
            }
        }
        found
    }

    #[test]
    fn every_route_of_the_api_names_a_permission_from_the_catalog() {
        let spec: Value = serde_json::from_str(&crate::openapi_json().unwrap()).unwrap();
        assert_eq!(undeclared(&spec), Vec::<String>::new());
    }

    #[test]
    fn a_route_without_a_known_permission_is_reported() {
        let spec = json!({"paths": {
            "/api/v1/items": {
                "get": {EXTENSION: "member"},
                "post": {EXTENSION: "signed_in"},
                "put": {},
                "patch": {EXTENSION: "items:write"},
                "parameters": [],
            },
            "/api/v1/items/search": {"get": {EXTENSION: "member"}},
        }});
        let found = undeclared(&spec);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].starts_with("PATCH /api/v1/items: declares Some(\"items:write\")"),
            "{found:?}"
        );
        assert!(
            found[1].starts_with("PUT /api/v1/items: declares None"),
            "{found:?}"
        );
    }

    #[test]
    fn the_names_are_unique() {
        let mut names: Vec<&str> = Permission::ALL
            .iter()
            .map(|permission| permission.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Permission::ALL.len());
    }
}
