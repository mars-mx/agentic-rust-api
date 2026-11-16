use secrecy::ExposeSecret;
use sqlx::{Connection, PgConnection};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::Migrate::load()?;
    telemetry::init(&config.log_level).map_err(anyhow::Error::from_boxed)?;
    let mut connection = PgConnection::connect(config.database_url.expose_secret()).await?;
    sqlx::query_file!("queries/group_roles.sql")
        .execute(&mut connection)
        .await?;
    sqlx::migrate!("../../crates/db/migrations")
        .run(&mut connection)
        .await?;
    let violations = sqlx::query_file_scalar!("queries/catalog_violations.sql")
        .fetch_all(&mut connection)
        .await?;
    for violation in &violations {
        tracing::error!(%violation, "catalog check failed");
    }
    if !violations.is_empty() {
        anyhow::bail!(
            "{} catalog check(s) failed after the migrations, so the new version must not start",
            violations.len()
        );
    }
    tracing::info!("migrations applied and the catalog checks passed");
    Ok(())
}
