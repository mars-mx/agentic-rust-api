use app::Database;
use axum::Router;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use clerk_rs::validators::jwks::JwksProvider;
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as Spec;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{Auth, require_auth};
use crate::catalog;
use crate::error::{AppError, ErrorCode, Problem};
use crate::me;

#[derive(OpenApi)]
#[openapi(components(schemas(Problem, ErrorCode)))]
struct ApiDoc;

fn api<J: JwksProvider + Send + Sync + 'static>(auth: Auth<J>) -> OpenApiRouter<Database> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(catalog::list_items, catalog::create_item))
        .routes(routes!(catalog::search_items))
        .routes(routes!(me::me))
        .route_layer(from_fn_with_state(auth, require_auth::<J>))
}

pub fn spec() -> Spec {
    api(Auth::clerk("", "", Vec::new())).into_openapi()
}

pub fn router<J: JwksProvider + Send + Sync + 'static>(
    auth: Auth<J>,
    database: Database,
) -> Router {
    let (api, _) = api(auth).split_for_parts();
    Router::new()
        .route("/health", get(health))
        .merge(api.with_state(database))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn not_found() -> AppError {
    AppError::NotFound
}

async fn method_not_allowed() -> AppError {
    AppError::MethodNotAllowed
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use serde_json::Value;
    use tower::ServiceExt;

    use app::Database;

    use super::router;
    use crate::auth::Auth;

    async fn call(method: Method, path: &str) -> (StatusCode, Option<String>, Vec<u8>) {
        let database = Database::lazy("postgres://localhost/unused").unwrap();
        let auth = Auth::clerk("", "https://clerk.example.test", Vec::new());
        let response = router(auth, database)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let media_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, media_type, body.to_vec())
    }

    #[tokio::test]
    async fn health_answers_ok_without_a_login() {
        let (status, _, body) = call(Method::GET, "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, Vec::<u8>::new());
    }

    #[tokio::test]
    async fn an_unknown_path_answers_not_found_as_problem_details() {
        let (status, media_type, body) = call(Method::GET, "/nothing-here").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(media_type.as_deref(), Some("application/problem+json"));
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "not_found");
    }

    #[tokio::test]
    async fn an_api_route_answers_unauthenticated_without_a_token() {
        let (status, media_type, body) = call(Method::GET, "/api/v1/me").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(media_type.as_deref(), Some("application/problem+json"));
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "unauthenticated");
    }

    #[tokio::test]
    async fn a_wrong_method_answers_method_not_allowed_as_problem_details() {
        let (status, media_type, body) = call(Method::POST, "/health").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(media_type.as_deref(), Some("application/problem+json"));
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "method_not_allowed");
    }
}
