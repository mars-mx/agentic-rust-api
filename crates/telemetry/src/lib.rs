use tracing_subscriber::EnvFilter;

pub fn init(level: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::try_new(level)?)
        .try_init()
}
