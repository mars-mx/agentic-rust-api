use std::net::SocketAddr;

use figment::providers::Env;
use figment::{Figment, Provider};
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiServer {
    pub listen_addr: SocketAddr,
    pub log_level: String,
    pub database_url: SecretString,
    pub clerk_secret_key: SecretString,
    pub clerk_frontend_api_url: String,
    pub clerk_authorized_parties: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Migrate {
    pub log_level: String,
    pub database_url: SecretString,
}

impl ApiServer {
    pub fn load() -> Result<Self, Box<figment::Error>> {
        from_provider(Env::raw().only(&[
            "listen_addr",
            "log_level",
            "database_url",
            "clerk_secret_key",
            "clerk_frontend_api_url",
            "clerk_authorized_parties",
        ]))
    }

    pub fn authorized_parties(&self) -> Vec<String> {
        self.clerk_authorized_parties
            .split(',')
            .map(str::trim)
            .filter(|party| !party.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

impl Migrate {
    pub fn load() -> Result<Self, Box<figment::Error>> {
        from_provider(Env::raw().only(&["log_level", "database_url"]))
    }
}

fn from_provider<T: DeserializeOwned>(provider: impl Provider) -> Result<T, Box<figment::Error>> {
    Figment::from(provider).extract().map_err(Box::new)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use figment::Figment;
    use secrecy::ExposeSecret;

    use super::{ApiServer, from_provider};

    fn complete() -> Figment {
        Figment::new()
            .merge(("listen_addr", "127.0.0.1:9000"))
            .merge(("log_level", "debug"))
            .merge(("database_url", "postgres://api@db/app"))
            .merge(("clerk_secret_key", "sk_test_x"))
            .merge(("clerk_frontend_api_url", "https://clerk.example.test"))
            .merge((
                "clerk_authorized_parties",
                "https://app.example.test, ,https://admin.example.test",
            ))
    }

    #[test]
    fn every_value_comes_from_the_environment() {
        let config: ApiServer = from_provider(complete()).unwrap();
        assert_eq!(
            config.listen_addr,
            "127.0.0.1:9000".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.database_url.expose_secret(), "postgres://api@db/app");
        assert_eq!(
            config.authorized_parties(),
            ["https://app.example.test", "https://admin.example.test"]
        );
    }

    #[test]
    fn a_secret_never_shows_in_debug_output() {
        let config: ApiServer = from_provider(complete()).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("sk_test_x"), "{debug}");
        assert!(!debug.contains("postgres://"), "{debug}");
    }

    #[test]
    fn a_missing_value_stops_the_load() {
        let error =
            from_provider::<ApiServer>(Figment::new().merge(("log_level", "debug"))).unwrap_err();
        assert!(error.to_string().contains("listen_addr"), "{error}");
    }

    #[test]
    fn a_malformed_address_stops_the_load() {
        let error = from_provider::<ApiServer>(complete().merge(("listen_addr", "not an address")))
            .unwrap_err();
        assert!(error.to_string().contains("listen_addr"), "{error}");
    }
}
