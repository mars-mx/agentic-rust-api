use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

const MEDIA_TYPE: &str = "application/problem+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthenticated,
    PermissionDenied,
    NotFound,
    MethodNotAllowed,
    InvalidRequest,
    QueryTimeout,
    Internal,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AppError {
    Unauthenticated,
    PermissionDenied,
    NotFound,
    MethodNotAllowed,
    InvalidRequest(String),
    QueryTimeout,
    Internal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Problem {
    #[serde(rename = "type")]
    #[schema(rename = "type")]
    kind: String,
    title: String,
    status: u16,
    detail: String,
    code: ErrorCode,
}

impl AppError {
    fn parts(&self) -> (StatusCode, ErrorCode, String) {
        match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthenticated,
                "Sign in to continue.".to_owned(),
            ),
            Self::PermissionDenied => (
                StatusCode::FORBIDDEN,
                ErrorCode::PermissionDenied,
                "You are not allowed to do this.".to_owned(),
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "Not found.".to_owned(),
            ),
            Self::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                ErrorCode::MethodNotAllowed,
                "This method is not allowed here.".to_owned(),
            ),
            Self::InvalidRequest(detail) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::InvalidRequest,
                detail.clone(),
            ),
            Self::QueryTimeout => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::QueryTimeout,
                "The request took too long. Please try again.".to_owned(),
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                "Something went wrong on our side.".to_owned(),
            ),
        }
    }

    pub fn problem(&self) -> Problem {
        let (status, code, detail) = self.parts();
        Problem {
            kind: "about:blank".to_owned(),
            title: status.canonical_reason().unwrap_or_default().to_owned(),
            status: status.as_u16(),
            detail,
            code,
        }
    }
}

impl From<app::Error> for AppError {
    fn from(error: app::Error) -> Self {
        match error {
            app::Error::Invalid(detail) => Self::InvalidRequest(detail),
            app::Error::Database(error) => {
                tracing::error!(%error, "database error");
                Self::Internal
            }
        }
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        Self::InvalidRequest(rejection.body_text())
    }
}

impl From<QueryRejection> for AppError {
    fn from(rejection: QueryRejection) -> Self {
        Self::InvalidRequest(rejection.body_text())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, ..) = self.parts();
        let mut response = (status, axum::Json(self.problem())).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(MEDIA_TYPE));
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use serde_json::{Value, json};

    use super::AppError;

    async fn answer(error: AppError) -> (StatusCode, String, Value) {
        let response = error.into_response();
        let status = response.status();
        let media_type = response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, media_type, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn an_error_answers_as_problem_details() {
        let (status, media_type, body) = answer(AppError::QueryTimeout).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(media_type, "application/problem+json");
        assert_eq!(
            body,
            json!({
                "type": "about:blank",
                "title": "Service Unavailable",
                "status": 503,
                "detail": "The request took too long. Please try again.",
                "code": "query_timeout",
            })
        );
    }

    #[tokio::test]
    async fn every_error_carries_its_status_in_body_and_header() {
        for error in [
            AppError::Unauthenticated,
            AppError::PermissionDenied,
            AppError::NotFound,
            AppError::MethodNotAllowed,
            AppError::InvalidRequest("limit must be at most 100".to_owned()),
            AppError::QueryTimeout,
            AppError::Internal,
        ] {
            let (status, media_type, body) = answer(error).await;
            assert_eq!(media_type, "application/problem+json");
            assert_eq!(body["status"], json!(status.as_u16()));
            assert!(
                body["detail"]
                    .as_str()
                    .is_some_and(|detail| !detail.is_empty())
            );
            assert!(body["code"].is_string());
        }
    }

    #[test]
    fn an_invalid_app_input_keeps_its_detail() {
        assert_eq!(
            AppError::from(app::Error::Invalid("title must not be empty".to_owned())),
            AppError::InvalidRequest("title must not be empty".to_owned())
        );
    }

    #[tokio::test]
    async fn an_invalid_request_keeps_its_detail() {
        let (status, _, body) = answer(AppError::InvalidRequest(
            "limit must be at most 100".to_owned(),
        ))
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["detail"], "limit must be at most 100");
    }
}
