use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ItemError {
    #[error("item {0} not found")]
    NotFound(Uuid),
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub items: Arc<[Item]>,
}

pub fn find(items: &[Item], id: Uuid) -> Result<&Item, ItemError> {
    items
        .iter()
        .find(|item| item.id == id)
        .ok_or(ItemError::NotFound(id))
}

#[tracing::instrument(skip_all)]
pub async fn show(State(state): State<AppState>, Path(id): Path<Uuid>) -> Json<Option<Item>> {
    Json(find(&state.items, id).ok().cloned())
}

pub fn router(state: AppState) -> Router {
    Router::new().route("/items/{id}", get(show)).with_state(state)
}

pub fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[cfg(test)]
mod tests {
    use super::{Item, find, new_id};

    #[test]
    fn finds_an_item_by_id() {
        let id = new_id();
        let items = vec![Item { id, name: "a".to_owned() }];
        assert_eq!(find(&items, id).unwrap().name, "a");
    }
}
