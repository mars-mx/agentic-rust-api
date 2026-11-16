use domain::catalog::Item;
use sqlx::PgConnection;

pub async fn insert_item(
    connection: &mut PgConnection,
    organization_id: &str,
    title: &str,
) -> Result<Item, sqlx::Error> {
    sqlx::query_file_as!(
        Item,
        "queries/catalog/insert_item.sql",
        organization_id,
        title
    )
    .fetch_one(connection)
    .await
}

pub async fn list_items(connection: &mut PgConnection) -> Result<Vec<Item>, sqlx::Error> {
    sqlx::query_file_as!(Item, "queries/catalog/list_items.sql")
        .fetch_all(connection)
        .await
}

pub async fn search_items(
    connection: &mut PgConnection,
    term: &str,
) -> Result<Vec<Item>, sqlx::Error> {
    sqlx::query_file_as!(Item, "queries/catalog/search_items.sql", term)
        .fetch_all(connection)
        .await
}
