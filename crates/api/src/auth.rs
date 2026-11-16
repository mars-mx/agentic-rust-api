use std::sync::Arc;
use std::time::Duration;

use app::Scope;
use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use clerk_rs::Clerk;
use clerk_rs::validators::authorizer::{ClerkError, ClerkJwt, validate_jwt};
use clerk_rs::validators::jwks::{
    JwksProvider, MemoryCacheJwksProvider, MemoryCacheJwksProviderOptions, RefreshOnUnknown,
};
use serde_json::Value;

use crate::error::AppError;

const UNKNOWN_KEY_REFETCH: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user_id: String,
    pub organization: Option<ActiveOrganization>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveOrganization {
    pub organization_id: String,
    pub role: String,
}

impl Identity {
    pub fn member_scope(&self) -> Result<Scope, AppError> {
        let organization = self
            .organization
            .as_ref()
            .ok_or(AppError::PermissionDenied)?;
        Ok(Scope {
            organization_id: organization.organization_id.clone(),
            user_id: self.user_id.clone(),
        })
    }
}

#[derive(Debug)]
pub struct Auth<J> {
    keys: Arc<J>,
    issuer: String,
    authorized_parties: Arc<[String]>,
}

impl<J> Clone for Auth<J> {
    fn clone(&self) -> Self {
        Self {
            keys: Arc::clone(&self.keys),
            issuer: self.issuer.clone(),
            authorized_parties: Arc::clone(&self.authorized_parties),
        }
    }
}

impl Auth<MemoryCacheJwksProvider> {
    pub fn clerk(
        secret_key: &str,
        frontend_api_url: &str,
        authorized_parties: Vec<String>,
    ) -> Self {
        let keys = MemoryCacheJwksProvider::new_with_options(
            Clerk::from_secret_key(secret_key),
            MemoryCacheJwksProviderOptions {
                expire_after: None,
                refresh_on_unknown: RefreshOnUnknown::Ratelimit(UNKNOWN_KEY_REFETCH),
            },
        );
        Self::new(keys, frontend_api_url, authorized_parties)
    }
}

impl<J: JwksProvider + Send + Sync> Auth<J> {
    fn new(keys: J, frontend_api_url: &str, authorized_parties: Vec<String>) -> Self {
        Self {
            keys: Arc::new(keys),
            issuer: frontend_api_url.trim_end_matches('/').to_owned(),
            authorized_parties: authorized_parties.into(),
        }
    }

    async fn verify(&self, authorization: Option<&str>) -> Result<Identity, Rejection> {
        let token = bearer(authorization).ok_or(Rejection::Missing)?;
        let claims =
            validate_jwt(token, Arc::clone(&self.keys))
                .await
                .map_err(|error| match error {
                    ClerkError::Unauthorized(_) => Rejection::Invalid,
                    ClerkError::InternalServerError(_) => Rejection::Keys,
                })?;
        if claims.iss != self.issuer {
            return Err(Rejection::Issuer);
        }
        if !claims
            .azp
            .as_ref()
            .is_some_and(|party| self.authorized_parties.contains(party))
        {
            return Err(Rejection::AuthorizedParty);
        }
        if claims.other.get("sts").and_then(Value::as_str) == Some("pending") {
            return Err(Rejection::Pending);
        }
        Ok(identity(claims))
    }
}

fn bearer(authorization: Option<&str>) -> Option<&str> {
    let (scheme, token) = authorization?.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn identity(claims: ClerkJwt) -> Identity {
    let organization = claims.other.get("o").and_then(|claim| {
        Some(ActiveOrganization {
            organization_id: claim
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())?
                .to_owned(),
            role: claim.get("rol").and_then(Value::as_str)?.to_owned(),
        })
    });
    Identity {
        user_id: claims.sub,
        organization,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    Missing,
    Invalid,
    Keys,
    Issuer,
    AuthorizedParty,
    Pending,
}

impl Rejection {
    fn reason(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Invalid => "invalid",
            Self::Keys => "jwks",
            Self::Issuer => "iss",
            Self::AuthorizedParty => "azp",
            Self::Pending => "pending",
        }
    }
}

pub async fn require_auth<J: JwksProvider + Send + Sync>(
    State(auth): State<Auth<J>>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    match auth.verify(authorization).await {
        Ok(identity) => {
            request.extensions_mut().insert(identity);
            Ok(next.run(request).await)
        }
        Err(rejection) => {
            tracing::info!(reason = rejection.reason(), "auth rejected");
            Err(AppError::Unauthenticated)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::middleware::from_fn_with_state;
    use axum::response::Response;
    use clerk_rs::validators::jwks::{JwksKey, JwksProvider, JwksProviderError};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::{Value, json};
    use tower::{ServiceBuilder, ServiceExt, service_fn};

    use super::{ActiveOrganization, Auth, Identity, Rejection, require_auth};

    const ISSUER: &str = "https://clerk.example.test";
    const APP: &str = "https://app.example.test";
    const KEY_ID: &str = "ins_test_key";
    const SIGNING_KEY: &str = include_str!("../tests/fixtures/test-signing-key.pem");
    const JWKS: &str = include_str!("../tests/fixtures/test-jwks.json");

    struct FixtureKeys;

    #[async_trait::async_trait]
    impl JwksProvider for FixtureKeys {
        type Error = JwksProviderError;

        async fn get_key(&self, kid: &str) -> Result<JwksKey, JwksProviderError> {
            let jwks: Value = serde_json::from_str(JWKS).unwrap();
            jwks["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| serde_json::from_value::<JwksKey>(key.clone()).unwrap())
                .find(|key| key.kid == kid)
                .ok_or(JwksProviderError::UnknownKey)
        }
    }

    fn auth() -> Auth<FixtureKeys> {
        Auth::new(FixtureKeys, &format!("{ISSUER}/"), vec![APP.to_owned()])
    }

    fn now() -> i64 {
        i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }

    fn claims() -> Value {
        json!({
            "azp": APP,
            "exp": now() + 60,
            "iat": now(),
            "nbf": now() - 5,
            "iss": ISSUER,
            "sid": "sess_1",
            "sub": "user_1",
            "v": 2,
            "o": {"id": "org_1", "rol": "admin", "slg": "acme"},
        })
    }

    fn sign_with(claims: &Value, kid: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_owned());
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(SIGNING_KEY.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn sign(claims: &Value) -> String {
        sign_with(claims, KEY_ID)
    }

    async fn verify(claims: &Value) -> Result<Identity, Rejection> {
        auth()
            .verify(Some(&format!("Bearer {}", sign(claims))))
            .await
    }

    fn with(changes: &[(&str, Value)]) -> Value {
        let mut claims = claims();
        for (name, value) in changes {
            claims[*name] = value.clone();
        }
        claims
    }

    fn without(name: &str) -> Value {
        let mut claims = claims();
        claims.as_object_mut().unwrap().remove(name);
        claims
    }

    #[tokio::test]
    async fn a_valid_session_token_yields_the_user_and_the_active_organization() {
        assert_eq!(
            verify(&claims()).await,
            Ok(Identity {
                user_id: "user_1".to_owned(),
                organization: Some(ActiveOrganization {
                    organization_id: "org_1".to_owned(),
                    role: "admin".to_owned(),
                }),
            })
        );
    }

    #[tokio::test]
    async fn only_an_identity_with_an_active_organization_has_a_member_scope() {
        let member = verify(&claims()).await.unwrap().member_scope().unwrap();
        assert_eq!(member.organization_id, "org_1");
        assert_eq!(member.user_id, "user_1");
        assert_eq!(
            verify(&without("o")).await.unwrap().member_scope(),
            Err(crate::error::AppError::PermissionDenied)
        );
    }

    #[tokio::test]
    async fn a_token_without_an_active_organization_yields_only_the_user() {
        for claims in [
            without("o"),
            with(&[("o", json!({"id": "", "rol": "admin"}))]),
        ] {
            assert_eq!(verify(&claims).await.unwrap().organization, None);
        }
    }

    #[tokio::test]
    async fn a_token_that_fails_a_check_is_rejected_with_its_reason() {
        let cases = [
            (with(&[("exp", json!(now() - 120))]), Rejection::Invalid),
            (with(&[("nbf", json!(now() + 120))]), Rejection::Invalid),
            (without("sub"), Rejection::Invalid),
            (
                with(&[("iss", json!("https://other.example.test"))]),
                Rejection::Issuer,
            ),
            (
                with(&[("azp", json!("https://evil.example.test"))]),
                Rejection::AuthorizedParty,
            ),
            (without("azp"), Rejection::AuthorizedParty),
            (with(&[("sts", json!("pending"))]), Rejection::Pending),
        ];
        for (claims, rejection) in cases {
            assert_eq!(verify(&claims).await, Err(rejection), "{claims}");
        }
    }

    #[tokio::test]
    async fn a_tampered_or_foreign_token_is_rejected() {
        let token = sign(&claims());
        let (head, rest) = token.split_once('.').unwrap();
        let (_, signature) = rest.split_once('.').unwrap();
        let forged_payload = sign(&with(&[("sub", json!("user_2"))]));
        let forged_payload = forged_payload.split('.').nth(1).unwrap();
        let forged = format!("{head}.{forged_payload}.{signature}");
        let unknown_key = sign_with(&claims(), "ins_other_key");
        for token in [forged, unknown_key, "not.a.token".to_owned()] {
            assert_eq!(
                auth().verify(Some(&format!("Bearer {token}"))).await,
                Err(Rejection::Invalid)
            );
        }
    }

    #[tokio::test]
    async fn only_a_bearer_header_with_a_token_is_read() {
        let token = sign(&claims());
        assert_eq!(
            auth()
                .verify(Some(&format!("bearer {token}")))
                .await
                .unwrap()
                .user_id,
            "user_1"
        );
        for header in [
            None,
            Some("Bearer"),
            Some("Bearer  "),
            Some(&*format!("Basic {token}")),
        ] {
            assert_eq!(
                auth().verify(header).await,
                Err(Rejection::Missing),
                "{header:?}"
            );
        }
    }

    async fn call(authorization: Option<String>) -> (StatusCode, Option<Identity>) {
        let service = ServiceBuilder::new()
            .layer(from_fn_with_state(auth(), require_auth::<FixtureKeys>))
            .service(service_fn(|request: Request<Body>| async move {
                let identity = request.extensions().get::<Identity>().cloned();
                let mut response = Response::new(Body::empty());
                response.extensions_mut().insert(identity);
                Ok::<_, Infallible>(response)
            }));
        let mut request = Request::builder().uri("/");
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        let response = service
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let identity = response
            .extensions()
            .get::<Option<Identity>>()
            .cloned()
            .flatten();
        (response.status(), identity)
    }

    #[tokio::test]
    async fn the_layer_hands_the_identity_to_the_handler() {
        let (status, identity) = call(Some(format!("Bearer {}", sign(&claims())))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(identity.unwrap().user_id, "user_1");
    }

    #[tokio::test]
    async fn the_layer_answers_unauthenticated_without_reaching_the_handler() {
        for authorization in [None, Some("Bearer not.a.token".to_owned())] {
            let (status, identity) = call(authorization).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(identity, None);
        }
    }
}
