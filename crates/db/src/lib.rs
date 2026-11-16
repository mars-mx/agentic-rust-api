pub mod catalog;

use sqlx::PgConnection;

pub async fn scope(
    connection: &mut PgConnection,
    organization_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/session/scope.sql", organization_id, user_id)
        .execute(connection)
        .await?;
    Ok(())
}
