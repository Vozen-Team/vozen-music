use crate::Error;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Unrestricted,
    Free,
    Plus,
    Premium,
}

impl Access {
    pub fn personal(self) -> bool {
        self != Self::Free
    }

    pub fn shared(self) -> bool {
        matches!(self, Self::Unrestricted | Self::Premium)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Unrestricted => "unrestricted",
            Self::Free => "free",
            Self::Plus => "plus",
            Self::Premium => "premium",
        }
    }
}

#[derive(Clone)]
pub struct Entitlements {
    http: reqwest::Client,
    endpoint: reqwest::Url,
    secret: String,
    votes: Option<(reqwest::Client, String)>,
}

pub const VOTE_REQUIRED: &str = "Vote for Vozen Music on Top.gg before enabling this radio mode.";
pub const VOTE_UNAVAILABLE: &str = "Music vote access could not be checked. Try again.";

#[derive(Deserialize)]
struct Vote {
    expires_at: String,
}

impl Vote {
    fn is_current(&self, now: chrono::DateTime<chrono::Utc>) -> Result<bool, chrono::ParseError> {
        Ok(chrono::DateTime::parse_from_rfc3339(&self.expires_at)? > now)
    }
}

#[cfg(test)]
#[test]
fn topgg_votes_expire_at_the_reported_boundary() {
    let vote = Vote {
        expires_at: "2026-11-05T00:00:00Z".into(),
    };
    let expiry = "2026-11-05T00:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap();
    assert!(
        vote.is_current(expiry - chrono::Duration::seconds(1))
            .unwrap()
    );
    assert!(!vote.is_current(expiry).unwrap());
}

#[derive(Deserialize)]
struct Resolution {
    product: String,
    subject_id: String,
    scope: String,
    plan: String,
    active: bool,
    expires_at_ms: Option<i64>,
}

impl Resolution {
    fn access(self, subject: &str, now_ms: i64) -> Result<Access, Error> {
        if self.product != "vozen" || self.subject_id != subject || self.scope != "guild" {
            return Err("Music access response did not match the request.".into());
        }
        if !self.active || self.expires_at_ms.is_some_and(|expiry| expiry <= now_ms) {
            return Ok(Access::Free);
        }
        match self.plan.as_str() {
            "free" => Ok(Access::Free),
            "plus" => Ok(Access::Plus),
            "premium" => Ok(Access::Premium),
            _ => Err("Music access response contained an unknown plan.".into()),
        }
    }
}

fn signature(secret: &str, timestamp: i64, nonce: &str, body: &[u8]) -> Result<String, Error> {
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(body));
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(format!("{timestamp}\n{nonce}\n{hash}").as_bytes());
    Ok(format!(
        "v1={}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

impl Entitlements {
    pub fn from_env() -> Result<Option<Self>, Error> {
        let vote_token = std::env::var("MUSIC_TOPGG_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
        let socket = std::env::var_os("MUSIC_ENTITLEMENT_SOCKET").filter(|v| !v.is_empty());
        let url = std::env::var("MUSIC_ENTITLEMENT_URL")
            .ok()
            .filter(|v| !v.is_empty());
        let secret = std::env::var("MUSIC_ENTITLEMENT_SECRET")
            .ok()
            .filter(|v| !v.is_empty());
        match (url, secret) {
            (None, None) if socket.is_none() && vote_token.is_none() => Ok(None),
            (Some(url), Some(secret)) => {
                let endpoint = reqwest::Url::parse(&url)?;
                if !matches!(endpoint.scheme(), "http" | "https")
                    || endpoint.host().is_none()
                    || !endpoint.username().is_empty()
                    || endpoint.password().is_some()
                    || endpoint.query().is_some()
                    || endpoint.fragment().is_some()
                    || endpoint.path() != "/internal/v1/entitlements/resolve"
                    || secret.len() < 32
                {
                    return Err("Invalid Music entitlement endpoint or secret.".into());
                }
                let builder = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .timeout(Duration::from_secs(3));
                let builder = if let Some(socket) = socket {
                    if endpoint.scheme() != "http" || endpoint.host_str() != Some("localhost") {
                        return Err(
                            "The Music entitlement socket requires an HTTP localhost URL.".into(),
                        );
                    }
                    #[cfg(unix)]
                    {
                        builder.unix_socket(std::path::PathBuf::from(socket))
                    }
                    #[cfg(not(unix))]
                    {
                        return Err("The Music entitlement socket requires Unix.".into());
                    }
                } else {
                    if endpoint.scheme() != "https"
                        && !matches!(
                            endpoint.host_str(),
                            Some("localhost" | "127.0.0.1" | "[::1]")
                        )
                    {
                        return Err("Use HTTPS or the private Music entitlement socket.".into());
                    }
                    builder
                };
                let http = builder.build()?;
                let votes = if let Some(token) = vote_token {
                    let client = reqwest::Client::builder()
                        .https_only(true)
                        .redirect(reqwest::redirect::Policy::none())
                        .timeout(Duration::from_secs(3))
                        .build()?;
                    Some((client, token))
                } else {
                    None
                };
                Ok(Some(Self {
                    http,
                    endpoint,
                    secret,
                    votes,
                }))
            }
            _ => Err("Configure both Music entitlement variables together.".into()),
        }
    }

    pub async fn resolve(&self, subject: u64, guild: u64) -> Result<Access, Error> {
        let subject = subject.to_string();
        let body = serde_json::to_vec(&serde_json::json!({
            "subject_id": subject,
            "guild_id": guild.to_string(),
        }))?;
        let timestamp = chrono::Utc::now().timestamp();
        let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 18]>());
        let signature = signature(&self.secret, timestamp, &nonce, &body)?;
        let result = self
            .http
            .post(self.endpoint.clone())
            .header("content-type", "application/json")
            .header("x-vozen-timestamp", timestamp.to_string())
            .header("x-vozen-nonce", nonce)
            .header("x-vozen-signature", signature)
            .body(body)
            .send()
            .await?
            .error_for_status()?
            .json::<Resolution>()
            .await?;
        result.access(&subject, chrono::Utc::now().timestamp_millis())
    }
}

impl crate::Bot {
    /// Whether this plan must hold a current Top.gg vote to switch to 24/7.
    pub fn radio_vote_applies(&self, access: Access) -> bool {
        !access.personal() && self.entitlements.as_ref().is_some_and(|e| e.votes.is_some())
    }

    pub async fn radio_vote_allowed(&self, subject: u64, access: Access) -> Result<bool, Error> {
        if access.personal() {
            return Ok(true);
        }
        let Some((http, token)) = self.entitlements.as_ref().and_then(|e| e.votes.as_ref()) else {
            return Ok(true);
        };
        current_vote(http, token, subject)
            .await
            .inspect_err(|error| log_lookup_failure("vote", error))
    }

    /// Access for starts nobody requested (restore, recovery, Auto-join).
    /// `None` means the resolver is unreachable: the radio must still start,
    /// so callers keep the server's saved settings without paid audio.
    pub async fn automatic_access(&self, subject: u64, guild: u64) -> Option<Access> {
        // `music_access` already logs the failure, at most once a minute.
        self.music_access(subject, guild).await.ok()
    }

    pub async fn music_access(&self, subject: u64, guild: u64) -> Result<Access, Error> {
        match &self.entitlements {
            None => Ok(Access::Unrestricted),
            Some(client) => client.resolve(subject, guild).await.map_err(|error| {
                log_lookup_failure("plan", &error);
                "Music plan access could not be checked. Try again.".into()
            }),
        }
    }
}

/// The member's Top.gg vote. Errors carry no URL, which would hold their Discord id.
async fn current_vote(http: &reqwest::Client, token: &str, subject: u64) -> Result<bool, Error> {
    let response = http
        .get(format!(
            "https://top.gg/api/v1/projects/@me/votes/{subject}?source=discord"
        ))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(false);
    }
    Ok(response
        .error_for_status()
        .map_err(|e| e.without_url())?
        .json::<Vote>()
        .await
        .map_err(|e| e.without_url())?
        .is_current(chrono::Utc::now())?)
}

/// Logs a failed plan or vote lookup at most once a minute per kind. Callers
/// pass errors without URLs; headers and request bodies are never logged.
pub(crate) fn log_lookup_failure(kind: &'static str, error: &Error) {
    static LAST: Mutex<std::collections::BTreeMap<&'static str, Instant>> =
        Mutex::new(std::collections::BTreeMap::new());
    let Ok(mut last) = LAST.lock() else { return };
    if last.get(kind).is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
        return;
    }
    last.insert(kind, Instant::now());
    eprintln!("Music {kind} lookup failed: {error}");
}

#[cfg(test)]
impl Entitlements {
    /// A configured resolver that refuses every connection.
    pub fn unreachable() -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: "http://127.0.0.1:9/internal/v1/entitlements/resolve"
                .parse()
                .unwrap(),
            secret: "test-only-music-central-signing-key".into(),
            votes: None,
        }
    }
}

#[cfg(test)]
#[tokio::test]
async fn automatic_starts_report_an_unknown_plan_when_the_resolver_is_down() {
    let bot = crate::test_bot(Some(Entitlements::unreachable()));
    assert!(bot.music_access(17, 23).await.is_err());
    assert_eq!(bot.automatic_access(17, 23).await, None);
    let unconfigured = crate::test_bot(None);
    assert_eq!(
        unconfigured.automatic_access(17, 23).await,
        Some(Access::Unrestricted)
    );
}

#[derive(Default)]
pub struct PlaybackQueue(Mutex<QueueState>);

#[derive(Default)]
struct QueueState {
    busy: bool,
    premium: VecDeque<oneshot::Sender<PlaybackSlot>>,
    standard: VecDeque<oneshot::Sender<PlaybackSlot>>,
    recent_station_commands: HashMap<(u64, u64), Instant>,
}

pub struct PlaybackSlot(Arc<PlaybackQueue>);

impl PlaybackQueue {
    pub fn check_station_cooldown(
        &self,
        guild: u64,
        actor: u64,
        access: Access,
    ) -> Result<(), Error> {
        let state = self.0.lock().map_err(|_| "Music queue unavailable.")?;
        let limit = if access.personal() { 2 } else { 5 };
        if state
            .recent_station_commands
            .get(&(guild, actor))
            .is_some_and(|last| last.elapsed() < Duration::from_secs(limit))
        {
            return Err("Wait a moment before selecting another station.".into());
        }
        Ok(())
    }

    pub fn mark_station_command(&self, guild: u64, actor: u64) {
        if let Ok(mut state) = self.0.lock() {
            state
                .recent_station_commands
                .retain(|_, last| last.elapsed() < Duration::from_secs(5));
            state
                .recent_station_commands
                .insert((guild, actor), Instant::now());
        }
    }

    pub async fn acquire(self: &Arc<Self>, priority: bool) -> Result<PlaybackSlot, Error> {
        let rx = {
            let mut state = self.0.lock().map_err(|_| "Music queue unavailable.")?;
            if !state.busy {
                state.busy = true;
                return Ok(PlaybackSlot(self.clone()));
            }
            state.premium.retain(|waiter| !waiter.is_closed());
            state.standard.retain(|waiter| !waiter.is_closed());
            if state.premium.len() + state.standard.len() >= 64 {
                return Err("Music is busy. Try again shortly.".into());
            }
            let (tx, rx) = oneshot::channel();
            if priority {
                state.premium.push_back(tx);
            } else {
                state.standard.push_back(tx);
            }
            rx
        };
        tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| "Music's playback queue timed out. Try again.")?
            .map_err(Into::into)
    }
}

impl Drop for PlaybackSlot {
    fn drop(&mut self) {
        let next = {
            let Ok(mut state) = self.0.0.lock() else {
                return;
            };
            let next = state
                .premium
                .pop_front()
                .or_else(|| state.standard.pop_front());
            state.busy = next.is_some();
            next
        };
        if let Some(next) = next {
            // If a waiter timed out, dropping its slot passes it to the next request.
            let _ = next.send(PlaybackSlot(self.0.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Poll},
    };

    fn result(plan: &str, subject: &str, expiry: i64) -> Resolution {
        Resolution {
            product: "vozen".into(),
            subject_id: subject.into(),
            scope: "guild".into(),
            plan: plan.into(),
            active: true,
            expires_at_ms: Some(expiry),
        }
    }

    #[test]
    fn plus_is_personal_and_premium_requires_a_guild_resolution() {
        let plus = result("plus", "17", 100).access("17", 99).unwrap();
        assert!(plus.personal());
        assert!(!plus.shared());
        assert!(
            result("premium", "17", 100)
                .access("17", 99)
                .unwrap()
                .shared()
        );
        let mut user_only = result("premium", "17", 100);
        user_only.scope = "user".into();
        assert!(user_only.access("17", 99).is_err());
        assert!(result("plus", "17", 100).access("18", 99).is_err());
        assert_eq!(
            result("plus", "17", 100).access("17", 100).unwrap(),
            Access::Free
        );
        assert_eq!(
            result("free", "18", 100).access("18", 99).unwrap(),
            Access::Free
        );
    }

    #[test]
    fn signs_exact_request_body_with_the_central_contract() {
        assert_eq!(
            signature(
                "test-only-music-central-signing-key",
                1730000000,
                "fixture-nonce",
                br#"{"guild_id":"23","subject_id":"17"}"#
            )
            .unwrap(),
            "v1=IVjn1s-7yYS9NlFh0Q2gNPRHNuwb5rme3-lL9RWRASg"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signed_socket_requests_keep_subject_and_server_isolated() {
        use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
        const SECRET: &str = "test-only-music-central-signing-key";
        let path =
            std::env::temp_dir().join(format!("music-entitlements-{}.sock", rand::random::<u64>()));
        let socket = tokio::net::UnixListener::bind(&path).unwrap();
        let app = Router::new().route("/internal/v1/entitlements/resolve", post(
            |headers: HeaderMap, body: Bytes| async move {
                let timestamp = headers["x-vozen-timestamp"].to_str().unwrap().parse().unwrap();
                let nonce = headers["x-vozen-nonce"].to_str().unwrap();
                assert_eq!(headers["x-vozen-signature"].to_str().unwrap(), signature(SECRET, timestamp, nonce, &body).unwrap());
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let plan = match (body["subject_id"].as_str(), body["guild_id"].as_str()) {
                    (_, Some("23")) => "premium",
                    (Some("17"), Some("24")) => "plus",
                    _ => "free",
                };
                axum::Json(serde_json::json!({
                    "product":"vozen", "subject_id":body["subject_id"], "scope":"guild",
                    "plan":plan, "active":true, "expires_at_ms":chrono::Utc::now().timestamp_millis()+60000,
                }))
            }
        ));
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = Entitlements {
            http: reqwest::Client::builder()
                .unix_socket(path.clone())
                .build()
                .unwrap(),
            endpoint: "http://localhost/internal/v1/entitlements/resolve"
                .parse()
                .unwrap(),
            secret: SECRET.into(),
            votes: None,
        };
        assert_eq!(client.resolve(17, 24).await.unwrap(), Access::Plus);
        assert_eq!(client.resolve(18, 24).await.unwrap(), Access::Free);
        assert_eq!(client.resolve(18, 23).await.unwrap(), Access::Premium);
        server.abort();
        let _ = server.await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn premium_goes_first_without_interrupting_active_playback_requests() {
        let queue = Arc::new(PlaybackQueue::default());
        let running = queue.acquire(false).await.unwrap();
        let mut standard = Box::pin(queue.acquire(false));
        let mut premium = Box::pin(queue.acquire(true));
        let mut ctx = Context::from_waker(std::task::Waker::noop());
        assert!(matches!(standard.as_mut().poll(&mut ctx), Poll::Pending));
        assert!(matches!(premium.as_mut().poll(&mut ctx), Poll::Pending));
        drop(running);
        assert!(matches!(standard.as_mut().poll(&mut ctx), Poll::Pending));
        let preferred = premium.await.unwrap();
        drop(preferred);
        drop(standard.await.unwrap());
        assert!(queue.acquire(false).await.is_ok());
    }
}
