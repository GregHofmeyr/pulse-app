//! HTTP client for the Pulse server. Only the Rust core talks to the network.

use std::time::Duration;

use pulse_protocol::ids::{ChannelId, MessageId, ServerId};
use pulse_protocol::rest::{
    Channel, EditMessageRequest, LoginRequest, Member, Message, RegisterRequest,
    SendMessageRequest, Server, SessionResponse, User, VoiceTokenResponse,
};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("wrong username or password")]
    Unauthorized,
    #[error("{0}")]
    Rejected(String),
    /// 5xx / 429: the server (or a proxy in front of it) is having trouble; worth retrying.
    #[error("the server is having trouble ({0})")]
    Server(String),
    #[error("can't reach the server: {0}")]
    Network(String),
}

impl serde::Serialize for ApiError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Plain http:// only for this machine; anything remote must be https (passwords + tokens).
pub fn check_server_url(url: &str) -> Result<(), ApiError> {
    let reject = |m: &str| Err(ApiError::Rejected(m.into()));
    if url.starts_with("https://") {
        return Ok(());
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return reject("server address must start with https://");
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        Ok(())
    } else {
        reject("use https:// for remote servers")
    }
}

#[derive(Clone)]
pub struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .expect("http client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
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
        if r.status().is_server_error() || r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ApiError::Server(r.status().to_string()));
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

    /// For endpoints that answer 204.
    async fn ok_empty(r: Result<reqwest::Response, reqwest::Error>) -> Result<(), ApiError> {
        let r = r.map_err(|e| ApiError::Network(e.to_string()))?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if r.status().is_server_error() || r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ApiError::Server(r.status().to_string()));
        }
        if !r.status().is_success() {
            let msg = r
                .json::<pulse_protocol::rest::ApiError>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unexpected server error".into());
            return Err(ApiError::Rejected(msg));
        }
        Ok(())
    }

    pub async fn join_server(&self, token: &str, server: ServerId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .post(format!("{}/servers/{server}/join", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn channels(&self, token: &str, server: ServerId) -> Result<Vec<Channel>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/channels", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn members(&self, token: &str, server: ServerId) -> Result<Vec<Member>, ApiError> {
        Self::parse(
            self.http
                .get(format!("{}/servers/{server}/members", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn messages(
        &self,
        token: &str,
        channel: ChannelId,
        before: Option<MessageId>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut req = self
            .http
            .get(format!("{}/channels/{channel}/messages", self.base))
            .bearer_auth(token);
        if let Some(b) = before {
            req = req.query(&[("before", b.to_string())]);
        }
        Self::parse(req.send().await).await
    }

    pub async fn send_message(
        &self,
        token: &str,
        channel: ChannelId,
        body: &SendMessageRequest,
    ) -> Result<Message, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/channels/{channel}/messages", self.base))
                .bearer_auth(token)
                .json(body)
                .send()
                .await,
        )
        .await
    }

    pub async fn edit_message(
        &self,
        token: &str,
        id: MessageId,
        content: &str,
    ) -> Result<Message, ApiError> {
        let body = EditMessageRequest {
            content: content.into(),
        };
        Self::parse(
            self.http
                .patch(format!("{}/messages/{id}", self.base))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await,
        )
        .await
    }

    pub async fn delete_message(&self, token: &str, id: MessageId) -> Result<(), ApiError> {
        Self::ok_empty(
            self.http
                .delete(format!("{}/messages/{id}", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
    }

    pub async fn voice_token(
        &self,
        token: &str,
        channel: ChannelId,
    ) -> Result<VoiceTokenResponse, ApiError> {
        Self::parse(
            self.http
                .post(format!("{}/voice/{channel}/token", self.base))
                .bearer_auth(token)
                .send()
                .await,
        )
        .await
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

    #[tokio::test]
    async fn join_then_post_and_page_messages() {
        let app = testing::spawn().await;
        let (_, owner) = testing::register(&app, "alex").await;
        let (_, token) = testing::register(&app, "sam").await;
        let s = testing::create_server(&app, &owner, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        api.join_server(&token, s.id).await.unwrap();
        let chans = api.channels(&token, s.id).await.unwrap();
        let general = chans
            .iter()
            .find(|c| c.name.as_deref() == Some("general"))
            .unwrap();
        for i in 0..55 {
            let req = SendMessageRequest {
                content: format!("m{i}"),
                reply_to_id: None,
                nonce: Some(format!("n{i}")),
            };
            api.send_message(&token, general.id, &req).await.unwrap();
        }
        let page1 = api.messages(&token, general.id, None).await.unwrap();
        assert_eq!(page1.len(), 50);
        let page2 = api
            .messages(&token, general.id, Some(page1[49].id))
            .await
            .unwrap();
        assert_eq!(page2.len(), 5);
        assert_eq!(api.members(&token, s.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn edit_and_delete_roundtrip() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let api = Api::new(&format!("http://{}", app.addr));
        let m = api
            .send_message(
                &token,
                g.id,
                &SendMessageRequest {
                    content: "tpyo".into(),
                    reply_to_id: None,
                    nonce: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            api.edit_message(&token, m.id, "typo")
                .await
                .unwrap()
                .content,
            "typo"
        );
        api.delete_message(&token, m.id).await.unwrap();
        assert!(api.messages(&token, g.id, None).await.unwrap()[0].deleted);
    }

    #[tokio::test]
    async fn voice_token_after_join() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let lounge = api
            .channels(&token, s.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.name.as_deref() == Some("Lounge"))
            .unwrap();
        let t = api.voice_token(&token, lounge.id).await.unwrap();
        assert!(!t.token.is_empty());
    }

    #[test]
    fn remote_http_refused() {
        assert!(check_server_url("http://localhost:7890").is_ok());
        assert!(check_server_url("http://127.0.0.1:7890").is_ok());
        assert!(check_server_url("http://[::1]:7890").is_ok());
        assert!(check_server_url("https://pulse.example.com").is_ok());
        assert!(matches!(
            check_server_url("http://pulse.example.com"),
            Err(ApiError::Rejected(_))
        ));
        assert!(matches!(
            check_server_url("ftp://x"),
            Err(ApiError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn timeout_on_unroutable_host() {
        let api = Api::new("http://10.255.255.1:9");
        let started = std::time::Instant::now();
        let err = api.login("a", "b").await.unwrap_err();
        assert!(matches!(err, ApiError::Network(_)), "{err:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(7));
    }
}
