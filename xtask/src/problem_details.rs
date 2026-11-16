use std::io::Write;

use serde_json::Value;

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];
const MEDIA_TYPE: &str = "application/problem+json";
const MEMBERS: [(&str, &str); 4] = [
    ("type", "string"),
    ("title", "string"),
    ("status", "integer"),
    ("detail", "string"),
];
const FIX: &str = "Every error answer is RFC 9457 Problem Details from the one IntoResponse in crates/api/src/error.rs. Declare it with the Problem Details schema that implementation returns.";

pub(crate) fn check(out: &mut impl Write) -> Result<bool, String> {
    let spec = api::openapi_json()
        .map_err(|error| format!("cannot render the Rust OpenAPI spec: {error}"))?;
    let spec: Value = serde_json::from_str(&spec)
        .map_err(|error| format!("the Rust OpenAPI spec is not JSON: {error}"))?;
    let found = violations(&spec);
    for violation in &found {
        writeln!(out, "{violation}")
            .map_err(|error| format!("cannot write the report: {error}"))?;
    }
    Ok(found.is_empty())
}

pub(crate) fn violations(spec: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let Some(paths) = spec.get("paths").and_then(Value::as_object) else {
        return found;
    };
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for (method, operation) in item {
            if !METHODS.contains(&method.as_str()) {
                continue;
            }
            let at = format!("{} {path}", method.to_uppercase());
            let mut errors = operation
                .get("responses")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter(|(status, _)| !status.starts_with('2'))
                .peekable();
            if errors.peek().is_none() {
                found.push(format!(
                    "{at}: declares no error response. Declare every error status the route answers, at least the ones AppError maps to. {FIX}"
                ));
            }
            for (status, response) in errors {
                if let Err(problem) = problem_details(spec, follow(spec, response)) {
                    found.push(format!("{at} > {status}: {problem}. {FIX}"));
                }
            }
        }
    }
    found
}

fn problem_details(spec: &Value, response: &Value) -> Result<(), String> {
    let content = response
        .get("content")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("has no content, expected {MEDIA_TYPE}"))?;
    let media_types: Vec<&str> = content.keys().map(String::as_str).collect();
    let schema = match (media_types.as_slice(), content.get(MEDIA_TYPE)) {
        ([MEDIA_TYPE], Some(entry)) => entry.get("schema").map(|schema| follow(spec, schema)),
        _ => {
            return Err(format!(
                "content is {}, expected exactly {MEDIA_TYPE}",
                media_types.join(", ")
            ));
        }
    }
    .ok_or_else(|| format!("{MEDIA_TYPE} has no schema"))?;
    for (member, kind) in MEMBERS {
        let declared = schema
            .get("properties")
            .and_then(|properties| properties.get(member))
            .map(|property| follow(spec, property))
            .and_then(|property| property.get("type"));
        if declared != Some(&Value::String(kind.to_owned())) {
            return Err(format!(
                "schema member {member} is {}, expected type {kind}",
                declared.map_or_else(|| "missing".to_owned(), ToString::to_string)
            ));
        }
    }
    let detail_required = schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| required.contains(&Value::String("detail".to_owned())));
    if detail_required {
        Ok(())
    } else {
        Err("schema does not list detail as required".to_owned())
    }
}

fn resolve<'a>(spec: &'a Value, value: &'a Value) -> &'a Value {
    value
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| spec.pointer(pointer))
        .unwrap_or(value)
}

fn follow<'a>(spec: &'a Value, value: &'a Value) -> &'a Value {
    let mut current = value;
    let mut seen = Vec::new();
    while let Some(reference) = current.get("$ref").and_then(Value::as_str) {
        if seen.contains(&reference) {
            break;
        }
        seen.push(reference);
        let next = resolve(spec, current);
        if std::ptr::eq(next, current) {
            break;
        }
        current = next;
    }
    current
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    fn problem() -> Value {
        json!({
            "type": "object",
            "required": ["type", "title", "status", "detail"],
            "properties": {
                "type": { "type": "string" },
                "title": { "type": "string" },
                "status": { "type": "integer", "format": "int32" },
                "detail": { "type": "string" },
                "instance": { "type": "string" },
            },
        })
    }

    fn spec(responses: &Value, problem: &Value) -> Value {
        json!({
            "paths": { "/api/v1/stores/{store_id}": {
                "parameters": [],
                "get": { "responses": responses },
            } },
            "components": {
                "schemas": { "Problem": problem },
                "responses": { "NotFound": {
                    "description": "not found",
                    "content": { "application/problem+json": { "schema": { "$ref": "#/components/schemas/Problem" } } },
                } },
            },
        })
    }

    fn ok() -> Value {
        json!({ "description": "ok", "content": { "application/json": { "schema": { "type": "object" } } } })
    }

    fn problem_response() -> Value {
        json!({ "description": "error", "content": { "application/problem+json": { "schema": { "$ref": "#/components/schemas/Problem" } } } })
    }

    fn violations_with(error: &Value, problem: &Value) -> Vec<String> {
        super::violations(&spec(&json!({ "200": ok(), "404": error }), problem))
    }

    fn reported(found: &[String], status: &str, problem: &str) -> bool {
        found.iter().any(|line| {
            line.starts_with(&format!(
                "GET /api/v1/stores/{{store_id}} > {status}: {problem}. "
            ))
        })
    }

    #[test]
    fn a_spec_without_paths_passes() {
        assert_eq!(
            super::violations(&json!({ "openapi": "3.1.0" })),
            Vec::<String>::new()
        );
    }

    #[test]
    fn problem_details_through_response_and_schema_refs_pass() {
        let responses = json!({
            "200": ok(),
            "404": { "$ref": "#/components/responses/NotFound" },
            "4XX": problem_response(),
            "default": problem_response(),
        });
        assert_eq!(
            super::violations(&spec(&responses, &problem())),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_operation_without_error_responses_is_reported() {
        let found = super::violations(&spec(&json!({ "200": ok() }), &problem()));
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("GET /api/v1/stores/{store_id}: declares no error response."));
    }

    #[test]
    fn an_error_without_content_is_reported() {
        let found = violations_with(&json!({ "description": "gone" }), &problem());
        assert!(reported(
            &found,
            "404",
            "has no content, expected application/problem+json"
        ));
    }

    #[test]
    fn an_error_as_plain_json_is_reported() {
        let error = json!({ "content": { "application/json": { "schema": problem() } } });
        let found = violations_with(&error, &problem());
        assert!(reported(
            &found,
            "404",
            "content is application/json, expected exactly application/problem+json"
        ));
    }

    #[test]
    fn a_second_media_type_next_to_problem_json_is_reported() {
        let error = json!({ "content": {
            "application/problem+json": { "schema": problem() },
            "text/plain": { "schema": { "type": "string" } },
        } });
        let found = violations_with(&error, &problem());
        assert!(reported(
            &found,
            "404",
            "content is application/problem+json, text/plain, expected exactly application/problem+json"
        ));
    }

    #[test]
    fn a_default_response_is_checked_too() {
        let responses = json!({ "200": ok(), "404": problem_response(), "default": { "content": { "text/plain": {} } } });
        let found = super::violations(&spec(&responses, &problem()));
        assert!(reported(
            &found,
            "default",
            "content is text/plain, expected exactly application/problem+json"
        ));
    }

    #[test]
    fn problem_json_without_schema_is_reported() {
        let error = json!({ "content": { "application/problem+json": {} } });
        let found = violations_with(&error, &problem());
        assert!(reported(
            &found,
            "404",
            "application/problem+json has no schema"
        ));
    }

    #[test]
    fn each_missing_member_is_reported() {
        for (member, kind) in super::MEMBERS {
            let mut schema = problem();
            if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
                properties.remove(member);
            }
            let found = violations_with(&problem_response(), &schema);
            assert!(
                reported(
                    &found,
                    "404",
                    &format!("schema member {member} is missing, expected type {kind}")
                ),
                "{member}: {found:?}"
            );
        }
    }

    #[test]
    fn a_member_with_the_wrong_type_is_reported() {
        let mut schema = problem();
        if let Some(status) = schema.pointer_mut("/properties/status") {
            *status = json!({ "type": "string" });
        }
        let found = violations_with(&problem_response(), &schema);
        assert!(reported(
            &found,
            "404",
            "schema member status is \"string\", expected type integer"
        ));
    }

    #[test]
    fn a_nullable_detail_is_reported() {
        let mut schema = problem();
        if let Some(detail) = schema.pointer_mut("/properties/detail") {
            *detail = json!({ "type": ["string", "null"] });
        }
        let found = violations_with(&problem_response(), &schema);
        assert!(reported(
            &found,
            "404",
            "schema member detail is [\"string\",\"null\"], expected type string"
        ));
    }

    #[test]
    fn an_optional_detail_is_reported() {
        let mut schema = problem();
        if let Some(required) = schema.get_mut("required") {
            *required = json!(["type", "title", "status"]);
        }
        let found = violations_with(&problem_response(), &schema);
        assert!(reported(
            &found,
            "404",
            "schema does not list detail as required"
        ));
    }

    #[test]
    fn a_ref_cycle_terminates_and_is_reported() {
        let mut document = spec(
            &json!({ "200": ok(), "404": problem_response() }),
            &problem(),
        );
        if let Some(schema) = document.pointer_mut("/components/schemas/Problem") {
            *schema = json!({ "$ref": "#/components/schemas/Problem" });
        }
        let found = super::violations(&document);
        assert!(reported(
            &found,
            "404",
            "schema member type is missing, expected type string"
        ));
    }
}
