use axum::Json;
use axum::extract::Extension;
use serde::Serialize;
use utoipa::ToSchema;

use crate::auth::Identity;
use crate::error::{AppError, Problem};

#[derive(Debug, Serialize, ToSchema)]
pub struct MeDto {
    pub user_id: String,
    pub organization_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "session",
    extensions(("x-permission" = json!("signed_in"))),
    responses(
        (status = 200, description = "The signed-in user and the active organization", body = MeDto),
        (status = 401, description = "No valid session token", body = Problem, content_type = "application/problem+json"),
    ),
)]
pub async fn me(Extension(identity): Extension<Identity>) -> Result<Json<MeDto>, AppError> {
    Ok(Json(MeDto {
        user_id: identity.user_id,
        organization_id: identity
            .organization
            .map(|organization| organization.organization_id),
    }))
}
