use api::auth::Auth;
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::ApiServer::load()?;
    telemetry::init(&config.log_level).map_err(anyhow::Error::from_boxed)?;
    let database = app::Database::connect(config.database_url.expose_secret()).await?;
    let auth = Auth::clerk(
        config.clerk_secret_key.expose_secret(),
        &config.clerk_frontend_api_url,
        config.authorized_parties(),
    );
    let mut terminate = signal(SignalKind::terminate())?;
    let listener = TcpListener::bind(config.listen_addr).await?;
    tracing::info!(addr = %config.listen_addr, "api-server started");
    axum::serve(listener, api::router::router(auth, database))
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        })
        .await?;
    tracing::info!("api-server stopped");
    Ok(())
}
