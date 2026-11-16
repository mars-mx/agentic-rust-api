pub mod auth;
pub mod catalog;
pub mod error;
pub mod me;
pub mod permission;
pub mod router;

pub fn openapi_json() -> Result<String, serde_json::Error> {
    router::spec().to_json()
}
