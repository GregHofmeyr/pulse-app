//! HTTP client for the Pulse server. Only the Rust core talks to the network.

use pulse_protocol::rest::{LoginRequest, RegisterRequest, Server, SessionResponse, User};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("wrong username or password")]
    Unauthorized,
    #[error("{0}")]
    Rejected(String),
    #[error("can't reach the server: {0}")]
    Network(String),
}

impl serde::Serialize for ApiError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Clone)]
pub struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn parse<T: DeserializeOwned>(
        r: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<T, ApiError> {
        let r = r.map_err(|e| ApiError::Network(e.to_string()))?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if !r.status().is_success() {
            let msg = r
                .json::<pulse_protocol::rest::ApiError>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unexpected server error".into());
            return Err(ApiError::Rejected(msg));
        }
        r.json().await.map_err(|e| ApiError::Network(e.to_string()))
    }

    pub async fn register(
        &self,
        invite_code: &str,
        username: &str,
        password: &str,
    ) -> Result<SessionResponse, ApiError> {
        let body = RegisterRequest {
            invite_code: invite_code.into(),
            username: username.into(),
            password: password.into(),
        };
        Self::parse(
            self.http
                .post(format!("{}/auth/register", self.base))
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<SessionResponse, ApiError> {
        let body = LoginRequest {
            username: username.into(),
            password: password.into(),
        };
        Self::parse(
            self.http
                .post(format!("{}/auth/login", self.base))
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn logout(&self, token: &str) -> Result<(), ApiError> {
        self.http
            .post(format!("{}/auth/logout", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(())
    }

    pub async fn me(&self, token: &str) -> Result<User, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/me", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn servers(&self, token: &str) -> Result<Vec<Server>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;

    #[tokio::test]
    async fn register_then_login_against_test_server_returns_user() {
        let app = testing::spawn().await;
        let code = testing::invite(&app).await;
        let api = Api::new(&format!("http://{}", app.addr));

        let reg = api.register(&code, "alex", "hunter2hunter2").await.unwrap();
        assert_eq!(reg.user.username, "alex");

        let login = api.login("alex", "hunter2hunter2").await.unwrap();
        assert_eq!(login.user.id, reg.user.id);
        assert_eq!(api.me(&login.token).await.unwrap().username, "alex");
    }

    #[tokio::test]
    async fn wrong_password_maps_to_unauthorized() {
        let app = testing::spawn().await;
        testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let err = api.login("alex", "nopenopenope").await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized), "{err:?}");
    }

    #[tokio::test]
    async fn lists_servers() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        testing::create_server(&app, &token, "Main Hangout").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let servers = api.servers(&token).await.unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Main Hangout");
    }
}
