use app::Database;
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use domain::catalog::Item;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::Identity;
use crate::error::{AppError, Problem};

#[derive(Debug, Serialize, ToSchema)]
pub struct ItemDto {
    pub id: Uuid,
    pub organization_id: Option<String>,
    pub title: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewItemDto {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
}

impl From<Item> for ItemDto {
    fn from(item: Item) -> Self {
        Self {
            id: item.id,
            organization_id: item.organization_id,
            title: item.title,
        }
    }
}

fn dtos(items: Vec<Item>) -> Json<Vec<ItemDto>> {
    Json(items.into_iter().map(ItemDto::from).collect())
}

#[utoipa::path(
    get,
    path = "/api/v1/items",
    tag = "catalog",
    extensions(("x-permission" = json!("member"))),
    responses(
        (status = 200, description = "The organization's own items and the shared ones", body = [ItemDto]),
        (status = 401, description = "No valid session token", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "No active organization", body = Problem, content_type = "application/problem+json"),
        (status = 500, description = "Server error", body = Problem, content_type = "application/problem+json"),
    ),
)]
pub async fn list_items(
    State(database): State<Database>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<ItemDto>>, AppError> {
    let scope = identity.member_scope()?;
    Ok(dtos(app::catalog::list_items(&database, &scope).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/items/search",
    tag = "catalog",
    params(("q" = String, Query, description = "Search term in web search syntax")),
    extensions(("x-permission" = json!("member"))),
    responses(
        (status = 200, description = "Own and shared items that match the search term", body = [ItemDto]),
        (status = 401, description = "No valid session token", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "No active organization", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Missing or empty search term", body = Problem, content_type = "application/problem+json"),
        (status = 500, description = "Server error", body = Problem, content_type = "application/problem+json"),
    ),
)]
pub async fn search_items(
    State(database): State<Database>,
    Extension(identity): Extension<Identity>,
    params: Result<Query<SearchParams>, QueryRejection>,
) -> Result<Json<Vec<ItemDto>>, AppError> {
    let Query(params) = params?;
    let scope = identity.member_scope()?;
    Ok(dtos(
        app::catalog::search_items(&database, &scope, &params.q).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/items",
    tag = "catalog",
    request_body = NewItemDto,
    extensions(("x-permission" = json!("member"))),
    responses(
        (status = 201, description = "The created item", body = ItemDto),
        (status = 401, description = "No valid session token", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "No active organization", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Invalid item", body = Problem, content_type = "application/problem+json"),
        (status = 500, description = "Server error", body = Problem, content_type = "application/problem+json"),
    ),
)]
pub async fn create_item(
    State(database): State<Database>,
    Extension(identity): Extension<Identity>,
    body: Result<Json<NewItemDto>, JsonRejection>,
) -> Result<(StatusCode, Json<ItemDto>), AppError> {
    let Json(body) = body?;
    let scope = identity.member_scope()?;
    let item = app::catalog::create_item(&database, &scope, &body.title).await?;
    Ok((StatusCode::CREATED, Json(ItemDto::from(item))))
}
