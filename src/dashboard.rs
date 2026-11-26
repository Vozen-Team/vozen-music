use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as RoutePath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    time::Instant,
};

const API: &str = "https://discord.com/api/v10";
const ACCOUNT_ORIGIN: &str = "https://vozen.org";
const ACCOUNT_CLIENT_ID: &str = "1537738930722443364";
const SESSION_COOKIE: &str = "__Secure-vozen_music";
const STATE_COOKIE: &str = "__Secure-vozen_music_oauth";
const SESSION_TTL: Duration = Duration::from_secs(3600);
const STATE_TTL: Duration = Duration::from_secs(600);
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Requests per client address and minute; static assets are not counted.
const CLIENT_REQUESTS: u32 = 300;
/// Generous ceiling across all clients; the OAuth callback is exempt from it.
const GLOBAL_REQUESTS: usize = 6000;
/// Bound on tracked client addresses; the oldest window is evicted when full.
const MAX_TRACKED_CLIENTS: usize = 10_000;
const MAX_PENDING: usize = 200;
const MAX_PENDING_PER_CLIENT: usize = 5;
const MAX_SESSIONS: usize = 1000;
const MAX_SESSIONS_PER_USER: usize = 5;
const PREVIEWS_PER_SESSION: usize = 2;

type WebResult<T> = Result<T, WebError>;

struct WebError(StatusCode, &'static str);

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error": self.1}))).into_response()
    }
}

fn unavailable() -> WebError {
    WebError(
        StatusCode::BAD_GATEWAY,
        "Discord is unavailable. Please try again.",
    )
}

#[derive(Clone)]
struct Login {
    user: UserInfo,
    token: String,
    csrf: String,
    expires: Instant,
    revoke_token: bool,
    /// Shared by every clone of this session; bounds concurrent audio previews.
    previews: Arc<tokio::sync::Semaphore>,
}

impl Login {
    fn new(user: UserInfo, token: String, expires: Instant, revoke_token: bool) -> Self {
        Self {
            user,
            token,
            csrf: nonce(),
            expires,
            revoke_token,
            previews: Arc::new(tokio::sync::Semaphore::new(PREVIEWS_PER_SESSION)),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct UserInfo {
    id: String,
    username: String,
    global_name: Option<String>,
    avatar: Option<String>,
    avatar_decoration_data: Option<AvatarDecoration>,
}

#[derive(Clone, Serialize, Deserialize)]
struct AvatarDecoration {
    asset: String,
}

/// Rate-limit key for one client: the address, or its /64 for IPv6 so that a
/// single host cannot rotate through its own prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ClientIp(IpAddr);

impl ClientIp {
    fn new(address: IpAddr) -> Self {
        match address.to_canonical() {
            IpAddr::V6(v6) => {
                let mut segments = v6.segments();
                segments[4..].fill(0);
                Self(IpAddr::V6(Ipv6Addr::from(segments)))
            }
            v4 => Self(v4),
        }
    }
}

/// Resolve the client address. In production the container port is published
/// only on the host loopback and Caddy forwards to it; per the Vozen Cloudflare
/// runbook Caddy overwrites CF-Connecting-IP / X-Forwarded-For with the client
/// address it validated. Those headers are trusted only from a loopback or
/// private peer (Caddy via the Docker bridge); any other peer is used as is.
fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> ClientIp {
    let peer = peer.map(|ip| ip.to_canonical());
    let trusted = peer.is_none_or(|ip| match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    });
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let forwarded = trusted
        .then(|| {
            header("cf-connecting-ip")
                .and_then(|value| value.trim().parse::<IpAddr>().ok())
                .or_else(|| {
                    // The right-most entry is the one appended by the local proxy.
                    header("x-forwarded-for")
                        .and_then(|value| value.rsplit(',').next())
                        .and_then(|value| value.trim().parse::<IpAddr>().ok())
                })
        })
        .flatten();
    ClientIp::new(forwarded.or(peer).unwrap_or(IpAddr::from([0, 0, 0, 0])))
}

/// Static, embedded or cached public files are cheap and not counted.
fn rate_exempt(path: &str) -> bool {
    [
        "/music/assets/",
        "/music/covers/",
        "/music/albums/",
        "/music/thumbnails/",
        "/music/css/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
        || matches!(
            path,
            "/music/health"
                | "/music/dashboard.css"
                | "/music/dashboard.js"
                | "/music/dashboard-i18n.js"
                | "/music/navigation.js"
                | "/music/server-picker.css"
                | "/music/avatar.png"
                | "/music/vozen.svg"
                | "/music/outfit.woff2"
        )
}

#[derive(Default)]
struct RequestLimits {
    global: VecDeque<Instant>,
    clients: HashMap<ClientIp, (Instant, u32)>,
}

impl RequestLimits {
    /// Count one request. `global` is false for routes that must never be
    /// blocked by other clients' traffic, such as the OAuth callback.
    fn check(&mut self, client: ClientIp, global: bool, now: Instant) -> bool {
        while self
            .global
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= RATE_WINDOW)
        {
            self.global.pop_front();
        }
        if global && self.global.len() >= GLOBAL_REQUESTS {
            return false;
        }
        if !self.clients.contains_key(&client) && self.clients.len() >= MAX_TRACKED_CLIENTS {
            self.clients
                .retain(|_, (start, _)| now.saturating_duration_since(*start) < RATE_WINDOW);
            if self.clients.len() >= MAX_TRACKED_CLIENTS
                && let Some(oldest) = self
                    .clients
                    .iter()
                    .min_by_key(|(_, (start, _))| *start)
                    .map(|(key, _)| *key)
            {
                self.clients.remove(&oldest);
            }
        }
        let (start, count) = self.clients.entry(client).or_insert((now, 0));
        if now.saturating_duration_since(*start) >= RATE_WINDOW {
            *start = now;
            *count = 0;
        }
        if *count >= CLIENT_REQUESTS {
            return false;
        }
        *count += 1;
        if global {
            self.global.push_back(now);
        }
        true
    }
}

struct Pending {
    expires: Instant,
    client: ClientIp,
}

#[derive(Default)]
struct AuthStore {
    pending: HashMap<String, Pending>,
    sessions: HashMap<String, Login>,
    limits: RequestLimits,
}

impl AuthStore {
    fn prune(&mut self, now: Instant) {
        self.pending.retain(|_, pending| pending.expires > now);
        self.sessions.retain(|_, login| login.expires > now);
    }

    /// Store a pending sign-in, evicting the client's (then everyone's) oldest
    /// state instead of refusing new sign-ins when a cap is reached.
    fn insert_pending(&mut self, value: String, client: ClientIp, now: Instant) {
        self.prune(now);
        let oldest = |pending: &HashMap<String, Pending>, own: bool| {
            pending
                .iter()
                .filter(|(_, p)| !own || p.client == client)
                .min_by_key(|(_, p)| p.expires)
                .map(|(key, _)| key.clone())
        };
        while self.pending.values().filter(|p| p.client == client).count() >= MAX_PENDING_PER_CLIENT
            && let Some(key) = oldest(&self.pending, true)
        {
            self.pending.remove(&key);
        }
        while self.pending.len() >= MAX_PENDING
            && let Some(key) = oldest(&self.pending, false)
        {
            self.pending.remove(&key);
        }
        self.pending.insert(
            value,
            Pending {
                expires: now + STATE_TTL,
                client,
            },
        );
    }

    /// Store a session, keeping at most `MAX_SESSIONS_PER_USER` per Discord user
    /// and evicting the soonest-expiring session when the store is full.
    fn insert_session(&mut self, key: String, login: Login, replaced: Option<&str>, now: Instant) {
        self.prune(now);
        if let Some(old) = replaced {
            self.sessions.remove(old);
        }
        let oldest = |sessions: &HashMap<String, Login>, user: Option<&str>| {
            sessions
                .iter()
                .filter(|(_, l)| user.is_none_or(|id| l.user.id == id))
                .min_by_key(|(_, l)| l.expires)
                .map(|(key, _)| key.clone())
        };
        while self
            .sessions
            .values()
            .filter(|l| l.user.id == login.user.id)
            .count()
            >= MAX_SESSIONS_PER_USER
            && let Some(old) = oldest(&self.sessions, Some(&login.user.id))
        {
            self.sessions.remove(&old);
        }
        while self.sessions.len() >= MAX_SESSIONS
            && let Some(old) = oldest(&self.sessions, None)
        {
            self.sessions.remove(&old);
        }
        self.sessions.insert(key, login);
    }

    fn consume_state(&mut self, value: &str, cookie: Option<&str>, now: Instant) -> bool {
        // A mismatched browser must not consume another browser's pending login.
        if cookie != Some(value) {
            return false;
        }
        self.pending
            .remove(value)
            .is_some_and(|pending| pending.expires > now)
    }
}

struct Dashboard {
    bot: Arc<Bot>,
    cache: Arc<Cache>,
    http: Arc<Http>,
    client: reqwest::Client,
    origin: String,
    client_id: String,
    secret: String,
    auth: Mutex<AuthStore>,
}

fn nonce() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then_some(value)
        })
}

fn set_cookie(response: &mut Response, name: &str, value: &str, seconds: u64) {
    let value =
        format!("{name}={value}; Path=/music; Max-Age={seconds}; HttpOnly; Secure; SameSite=Lax");
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&value).expect("generated cookie"),
    );
}

fn origin_valid(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && !value.ends_with('/')
    })
}

fn csrf_valid(headers: &HeaderMap, origin: &str, csrf: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        == Some(origin)
        && headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            == Some(csrf)
}

async fn guarded(
    State(state): State<Arc<Dashboard>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let client = client_ip(request.headers(), peer);
    request.extensions_mut().insert(client);
    let path = request.uri().path();
    let allowed = rate_exempt(path) || {
        let global = path != "/music/auth/callback";
        state
            .auth
            .lock()
            .await
            .limits
            .check(client, global, Instant::now())
    };
    let mut response = if allowed {
        next.run(request).await
    } else {
        WebError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Wait a minute and try again.",
        )
        .into_response()
    };
    let headers = response.headers_mut();
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    for (name, value) in [
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https://cdn.discordapp.com; font-src 'self'; connect-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    ] {
        headers.insert(
            axum::http::HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}

impl Dashboard {
    async fn music_access(
        &self,
        guild: GuildId,
        login: &Login,
    ) -> WebResult<crate::access::Access> {
        let subject = parse_id(&login.user.id)?;
        self.bot
            .music_access(subject, guild.get())
            .await
            .map_err(|_| {
                WebError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Music plan access could not be checked. Try again.",
                )
            })
    }

    async fn login(&self, headers: &HeaderMap, write: bool) -> WebResult<Login> {
        let key = cookie(headers, SESSION_COOKIE).ok_or(WebError(
            StatusCode::UNAUTHORIZED,
            "Sign in with Discord to continue.",
        ))?;
        let login = self
            .auth
            .lock()
            .await
            .sessions
            .get(key)
            .filter(|login| login.expires > Instant::now())
            .cloned()
            .ok_or(WebError(
                StatusCode::UNAUTHORIZED,
                "Your session expired. Sign in again.",
            ))?;
        if write && !csrf_valid(headers, &self.origin, &login.csrf) {
            return Err(WebError(
                StatusCode::FORBIDDEN,
                "This request could not be verified. Reload the dashboard.",
            ));
        }
        Ok(login)
    }

    async fn fetch<T: DeserializeOwned>(&self, path: &str, token: &str) -> WebResult<T> {
        let response = self
            .client
            .get(format!("{API}{path}"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(WebError(
                StatusCode::UNAUTHORIZED,
                "Discord authorization expired. Sign in again.",
            ));
        }
        response
            .error_for_status()
            .map_err(|_| unavailable())?
            .json()
            .await
            .map_err(|_| unavailable())
    }

    async fn manager(&self, guild: GuildId, login: &Login) -> WebResult<PartialGuild> {
        Ok(self.manager_member(guild, login).await?.0)
    }

    /// Like `manager`, also returning the member for channel permission checks.
    async fn manager_member(
        &self,
        guild: GuildId,
        login: &Login,
    ) -> WebResult<(PartialGuild, Member)> {
        if self.bot.guild.is_some_and(|allowed| allowed != guild)
            || self.cache.guild(guild).is_none()
        {
            return Err(WebError(
                StatusCode::NOT_FOUND,
                "Vozen Music is not available in this server.",
            ));
        }
        let user = login
            .user
            .id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(unavailable)?;
        let member = guild
            .member(&self.http, UserId::new(user))
            .await
            .map_err(|_| {
                WebError(
                    StatusCode::FORBIDDEN,
                    "Server membership could not be verified.",
                )
            })?;
        let server = guild
            .to_partial_guild(&self.http)
            .await
            .map_err(|_| unavailable())?;
        let mut permissions = server
            .roles
            .get(&RoleId::new(guild.get()))
            .map(|role| role.permissions)
            .unwrap_or_default();
        for role in &member.roles {
            permissions |= server
                .roles
                .get(role)
                .map(|role| role.permissions)
                .unwrap_or_default();
        }
        if server.owner_id.get() != user
            && !permissions.intersects(Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR)
        {
            return Err(WebError(
                StatusCode::FORBIDDEN,
                "You need Manage Server permission to use this dashboard.",
            ));
        }
        Ok((server, member))
    }
}

async fn authorize(
    State(state): State<Arc<Dashboard>>,
    axum::Extension(client): axum::Extension<ClientIp>,
) -> WebResult<Response> {
    let value = nonce();
    state
        .auth
        .lock()
        .await
        .insert_pending(value.clone(), client, Instant::now());
    let mut url =
        reqwest::Url::parse("https://discord.com/oauth2/authorize").expect("constant URL");
    url.query_pairs_mut().extend_pairs([
        ("client_id", state.client_id.as_str()),
        ("response_type", "code"),
        ("scope", "identify guilds"),
        (
            "redirect_uri",
            &format!("{}/music/auth/callback", state.origin),
        ),
        ("state", value.as_str()),
    ]);
    let mut response = Redirect::to(url.as_str()).into_response();
    set_cookie(&mut response, STATE_COOKIE, &value, STATE_TTL.as_secs());
    Ok(response)
}

#[derive(Deserialize)]
struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    iss: Option<String>,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
    token_type: String,
}

async fn callback(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> WebResult<Response> {
    let value = query.state.as_deref().ok_or(WebError(
        StatusCode::BAD_REQUEST,
        "Missing sign-in state. Start sign-in again.",
    ))?;
    if !state
        .auth
        .lock()
        .await
        .consume_state(value, cookie(&headers, STATE_COOKIE), Instant::now())
    {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Sign-in expired or belongs to another browser. Start sign-in again.",
        ));
    }
    if query
        .iss
        .as_deref()
        .is_some_and(|issuer| issuer != "https://discord.com")
    {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Invalid authorization issuer.",
        ));
    }
    if query.error.is_some() {
        let mut response = Redirect::to("/music?login=cancelled").into_response();
        set_cookie(&mut response, STATE_COOKIE, "", 0);
        return Ok(response);
    }
    let code = query.code.ok_or(WebError(
        StatusCode::BAD_REQUEST,
        "Missing authorization code. Start sign-in again.",
    ))?;
    let token: Token = state
        .client
        .post("https://discord.com/api/oauth2/token")
        .basic_auth(&state.client_id, Some(&state.secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            (
                "redirect_uri",
                &format!("{}/music/auth/callback", state.origin),
            ),
        ])
        .send()
        .await
        .map_err(|_| unavailable())?
        .error_for_status()
        .map_err(|_| unavailable())?
        .json()
        .await
        .map_err(|_| unavailable())?;
    if !token.token_type.eq_ignore_ascii_case("bearer") || token.expires_in == 0 {
        return Err(unavailable());
    }
    let user: UserInfo = state.fetch("/users/@me", &token.access_token).await?;
    let key = nonce();
    let ttl = SESSION_TTL.min(Duration::from_secs(token.expires_in));
    state.auth.lock().await.insert_session(
        key.clone(),
        Login::new(user, token.access_token, Instant::now() + ttl, true),
        cookie(&headers, SESSION_COOKIE),
        Instant::now(),
    );
    let mut response = Redirect::to("/music").into_response();
    set_cookie(&mut response, STATE_COOKIE, "", 0);
    set_cookie(&mut response, SESSION_COOKIE, &key, ttl.as_secs());
    Ok(response)
}

async fn logout(State(state): State<Arc<Dashboard>>, headers: HeaderMap) -> WebResult<Response> {
    let login = state.login(&headers, true).await?;
    if let Some(key) = cookie(&headers, SESSION_COOKIE) {
        state.auth.lock().await.sessions.remove(key);
    }
    // Local logout is immediate, even when Discord's revocation endpoint is unavailable.
    if login.revoke_token {
        let _ = state
            .client
            .post("https://discord.com/api/oauth2/token/revoke")
            .basic_auth(&state.client_id, Some(&state.secret))
            .form(&[("token", login.token)])
            .send()
            .await;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(&mut response, SESSION_COOKIE, "", 0);
    Ok(response)
}

fn account_cors(headers: &HeaderMap) -> WebResult<HeaderMap> {
    if headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        != Some(ACCOUNT_ORIGIN)
    {
        return Err(WebError(StatusCode::FORBIDDEN, "Untrusted account origin."));
    }
    let mut result = HeaderMap::new();
    result.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static(ACCOUNT_ORIGIN),
    );
    result.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    result.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    result.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    result.insert(header::VARY, HeaderValue::from_static("Origin"));
    Ok(result)
}

async fn account_preflight(headers: HeaderMap) -> WebResult<Response> {
    Ok((account_cors(&headers)?, StatusCode::NO_CONTENT).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountToken {
    token: String,
}

fn account_authorized(value: &serde_json::Value) -> Option<Duration> {
    if value.pointer("/application/id")?.as_str()? != ACCOUNT_CLIENT_ID {
        return None;
    }
    let scopes = value.get("scopes")?.as_array()?;
    if !["identify", "guilds"]
        .iter()
        .all(|scope| scopes.iter().any(|value| value.as_str() == Some(scope)))
    {
        return None;
    }
    let expires = chrono::DateTime::parse_from_rfc3339(value.get("expires")?.as_str()?).ok()?;
    let seconds = (expires.timestamp() - chrono::Utc::now().timestamp())
        .try_into()
        .ok()?;
    let ttl = SESSION_TTL.min(Duration::from_secs(seconds));
    (!ttl.is_zero()).then_some(ttl)
}

async fn account_bridge(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Json(body): Json<AccountToken>,
) -> WebResult<Response> {
    let cors = account_cors(&headers)?;
    if body.token.len() < 20
        || body.token.len() > 4096
        || body.token.chars().any(char::is_whitespace)
    {
        return Err(WebError(
            StatusCode::UNAUTHORIZED,
            "Invalid account session.",
        ));
    }
    let oauth: serde_json::Value = state.fetch("/oauth2/@me", &body.token).await?;
    let ttl = account_authorized(&oauth).ok_or(WebError(
        StatusCode::UNAUTHORIZED,
        "Account authorization is not valid for Vozen.",
    ))?;
    let user: UserInfo = state.fetch("/users/@me", &body.token).await?;
    if oauth.pointer("/user/id").and_then(|id| id.as_str()) != Some(user.id.as_str()) {
        return Err(WebError(
            StatusCode::UNAUTHORIZED,
            "Account identity could not be verified.",
        ));
    }
    let key = nonce();
    state.auth.lock().await.insert_session(
        key.clone(),
        Login::new(user, body.token, Instant::now() + ttl, false),
        cookie(&headers, SESSION_COOKIE),
        Instant::now(),
    );
    let mut response = (cors, StatusCode::NO_CONTENT).into_response();
    set_cookie(&mut response, SESSION_COOKIE, &key, ttl.as_secs());
    Ok(response)
}

async fn account_logout(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> WebResult<Response> {
    let cors = account_cors(&headers)?;
    if let Some(key) = cookie(&headers, SESSION_COOKIE) {
        state.auth.lock().await.sessions.remove(key);
    }
    let mut response = (cors, StatusCode::NO_CONTENT).into_response();
    set_cookie(&mut response, SESSION_COOKIE, "", 0);
    Ok(response)
}

async fn me(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    Ok(Json(
        serde_json::json!({"user": login.user, "csrf": login.csrf}),
    ))
}

#[derive(Deserialize, Serialize)]
struct GuildInfo {
    id: String,
    name: String,
    icon: Option<String>,
    owner: bool,
    permissions: String,
}

fn can_manage(owner: bool, permissions: &str) -> bool {
    owner
        || permissions
            .parse::<u64>()
            .is_ok_and(|bits| bits & (8 | 32) != 0)
}

async fn guilds(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    let mut managed = Vec::new();
    let mut after = String::new();
    for _ in 0..10 {
        let path = if after.is_empty() {
            "/users/@me/guilds?limit=200".to_owned()
        } else {
            format!("/users/@me/guilds?limit=200&after={after}")
        };
        let page: Vec<GuildInfo> = state.fetch(&path, &login.token).await?;
        let last = page.last().map(|guild| guild.id.clone());
        let complete = page.len() < 200;
        managed.extend(
            page.into_iter()
                .filter(|guild| {
                    can_manage(guild.owner, &guild.permissions)
                        && guild
                            .id
                            .parse::<u64>()
                            .ok()
                            .filter(|id| *id > 0)
                            .is_some_and(|id| {
                                let id = GuildId::new(id);
                                state.bot.guild.is_none_or(|allowed| allowed == id)
                                    && state.cache.guild(id).is_some()
                            })
                })
                .map(|guild| serde_json::json!({"id": guild.id, "name": guild.name, "icon": guild.icon})),
        );
        if complete {
            return Ok(Json(serde_json::json!({"guilds": managed})));
        }
        after = last.ok_or_else(unavailable)?;
        if after.parse::<u64>().is_err() {
            return Err(unavailable());
        }
    }
    Err(WebError(
        StatusCode::BAD_GATEWAY,
        "Discord returned too many servers. Please try again.",
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    access: vozen_music::skip::SkipAccess,
    mode: vozen_music::skip::SkipMode,
    majority_scope: vozen_music::skip::MajorityScope,
    moderator_roles: Vec<String>,
}

impl From<SkipSettings> for Policy {
    fn from(settings: SkipSettings) -> Self {
        Self {
            access: settings.access,
            mode: settings.mode,
            majority_scope: settings.majority_scope,
            moderator_roles: settings
                .moderator_roles
                .iter()
                .map(u64::to_string)
                .collect(),
        }
    }
}

fn parse_id(value: &str) -> WebResult<u64> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WebError(StatusCode::BAD_REQUEST, "Invalid Discord ID."));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(WebError(StatusCode::BAD_REQUEST, "Invalid Discord ID."))
}

fn validate_policy(
    policy: Policy,
    guild: GuildId,
    available: &HashMap<RoleId, Role>,
) -> WebResult<SkipSettings> {
    if policy.moderator_roles.len() > 50 {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Choose at most 50 moderator roles.",
        ));
    }
    let mut roles = std::collections::BTreeSet::new();
    for id in policy.moderator_roles {
        let id = parse_id(&id)?;
        if id == guild.get() || !available.contains_key(&RoleId::new(id)) {
            return Err(WebError(
                StatusCode::BAD_REQUEST,
                "A selected role was deleted or is not a moderator role. Reload and choose again.",
            ));
        }
        roles.insert(id);
    }
    Ok(SkipSettings {
        access: policy.access,
        mode: policy.mode,
        majority_scope: policy.majority_scope,
        moderator_roles: roles,
    })
}

async fn settings(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    let guild = GuildId::new(parse_id(&value)?);
    let server = state.manager(guild, &login).await?;
    let stored = state
        .bot
        .settings
        .lock()
        .await
        .db
        .settings(guild.get())
        .map_err(storage_error)?;
    let revision = stored.revision;
    let policy = Policy::from(stored.skip);
    let mut roles: Vec<_> = server.roles.values().filter(|role| role.id.get() != guild.get())
        .map(|role| serde_json::json!({"id": role.id.to_string(), "name": role.name, "position": role.position})).collect();
    roles.sort_by_key(|role| std::cmp::Reverse(role["position"].as_i64().unwrap_or(0)));
    Ok(Json(
        serde_json::json!({"id": guild.to_string(), "name": server.name, "settings": policy, "roles": roles,"revision":revision}),
    ))
}

async fn save(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
    Json(policy): Json<Policy>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, true).await?;
    let guild = GuildId::new(parse_id(&value)?);
    let server = state.manager(guild, &login).await?;
    let policy = validate_policy(policy, server.id, &server.roles)?;
    let expected = headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or(WebError(
            StatusCode::PRECONDITION_REQUIRED,
            "Reload the dashboard before saving these settings.",
        ))?;
    let session = {
        let sessions = state.bot.sessions.lock().await;
        let mut store = state.bot.settings.lock().await;
        if store
            .db
            .settings(guild.get())
            .map_err(storage_error)?
            .revision
            != expected
        {
            return Err(WebError(
                StatusCode::CONFLICT,
                "Settings changed. Reload before saving.",
            ));
        }
        store.set(guild.get(), policy.clone()).map_err(|_| {
            WebError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Settings could not be saved. Previous settings remain active. Try again.",
            )
        })?;
        if let Some(session) = sessions.get(&guild) {
            *session.policy.lock().map_err(|_| unavailable())? = policy.clone();
            session
                .reset_votes(session.generation.load(Ordering::SeqCst))
                .map_err(|_| unavailable())?;
        }
        sessions.get(&guild).cloned()
    };
    if let Some(session) = session {
        player::refresh(&state.http, &state.bot.catalog, &session, false).await;
    }
    Ok(Json(serde_json::json!({"settings": Policy::from(policy)})))
}

include!("dashboard_expansion.rs");

fn preview_range(value: Option<&str>, length: u64) -> Option<(u64, u64)> {
    if length == 0 {
        return None;
    }
    let Some(value) = value else {
        return Some((0, length - 1));
    };
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if !start.bytes().all(|b| b.is_ascii_digit()) || !end.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok().filter(|v| *v > 0)?;
        return Some((length.saturating_sub(suffix), length - 1));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>().ok()?.min(length - 1)
    };
    (start < length && start <= end).then_some((start, end))
}

/// Keeps a preview slot reserved for as long as the response body is alive.
struct PermitReader<R> {
    inner: R,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for PermitReader<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

async fn recording_preview(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(id): RoutePath<String>,
) -> WebResult<Response> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let login = state.login(&headers, false).await?;
    // Held by the response body until the stream finishes or is dropped.
    let permit = login.previews.clone().try_acquire_owned().map_err(|_| {
        WebError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many previews are playing. Stop one and try again.",
        )
    })?;
    let track = state
        .bot
        .catalog
        .stations
        .iter()
        .flat_map(|s| &s.tracks)
        .find(|t| t.id == id)
        .ok_or(WebError(StatusCode::NOT_FOUND, "Recording not found."))?;
    let mut file = tokio::fs::File::open(&track.file)
        .await
        .map_err(|_| unavailable())?;
    let length = file.metadata().await.map_err(|_| unavailable())?.len();
    let range = headers.get(header::RANGE);
    let selected = if let Some(range) = range {
        range
            .to_str()
            .ok()
            .and_then(|value| preview_range(Some(value), length))
    } else {
        preview_range(None, length)
    };
    let Some((start, end)) = selected else {
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{length}"))],
        )
            .into_response());
    };
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| unavailable())?;
    let mime = match track.file.extension().and_then(|e| e.to_str()) {
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    };
    let size = end - start + 1;
    let stream = tokio_util::io::ReaderStream::new(PermitReader {
        inner: file.take(size),
        _permit: permit,
    });
    let mut response = axum::body::Body::from_stream(stream).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&size.to_string()).unwrap(),
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    if range.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{length}")).unwrap(),
        );
    }
    Ok(response)
}

async fn album_cover(RoutePath(name): RoutePath<String>) -> WebResult<Response> {
    let bytes: &[u8] = match name.as_str() {
        "lofi-v1.webp" => include_bytes!("../web/albums/lofi-v1.webp"),
        "ambient-v1.webp" => include_bytes!("../web/albums/ambient-v1.webp"),
        "jazz-v1.webp" => include_bytes!("../web/albums/jazz-v1.webp"),
        "electronic-v1.webp" => include_bytes!("../web/albums/electronic-v1.webp"),
        "piano-v1.webp" => include_bytes!("../web/albums/piano-v1.webp"),
        "cinematic-v1.webp" => include_bytes!("../web/albums/cinematic-v1.webp"),
        "rock-v1.webp" => include_bytes!("../web/albums/rock-v1.webp"),
        "night-v1.webp" => include_bytes!("../web/albums/night-v1.webp"),
        "rain-v1.webp" => include_bytes!("../web/albums/rain-v1.webp"),
        "sleep-v1.webp" => include_bytes!("../web/albums/sleep-v1.webp"),
        "my-v1.webp" => include_bytes!("../web/albums/my-v1.webp"),
        "synthwave-v1.webp" => include_bytes!("../web/albums/synthwave-v1.webp"),
        "acoustic-v1.webp" => include_bytes!("../web/albums/acoustic-v1.webp"),
        "chiptune-v1.webp" => include_bytes!("../web/albums/chiptune-v1.webp"),
        "chill-house-v1.webp" => include_bytes!("../web/albums/chill-house-v1.webp"),
        "drum-and-bass-v1.webp" => include_bytes!("../web/albums/drum-and-bass-v1.webp"),
        "bossa-nova-v1.webp" => include_bytes!("../web/albums/bossa-nova-v1.webp"),
        _ => return Err(WebError(StatusCode::NOT_FOUND, "Cover not found.")),
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response())
}

static TRACK_THUMBNAILS: std::sync::LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static THUMBNAIL_JOBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

async fn track_thumbnail(
    State(state): State<Arc<Dashboard>>,
    RoutePath(name): RoutePath<String>,
) -> WebResult<Response> {
    let original = name
        .strip_suffix(".webp")
        .ok_or(WebError(StatusCode::NOT_FOUND, "Cover not found."))?;
    let original = format!("{original}.png");
    let path = state
        .bot
        .catalog
        .stations
        .iter()
        .flat_map(|s| &s.tracks)
        .filter_map(|t| t.cover.as_ref())
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(original.as_str()))
        .cloned()
        .ok_or(WebError(StatusCode::NOT_FOUND, "Cover not found."))?;
    let _permit = THUMBNAIL_JOBS.acquire().await.map_err(|_| unavailable())?;
    let cached = TRACK_THUMBNAILS.lock().await.get(&name).cloned();
    let bytes = if let Some(bytes) = cached {
        bytes
    } else {
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, vozen_music::Error> {
            let image = image::open(path)?.thumbnail(128, 128);
            let mut out = std::io::Cursor::new(Vec::new());
            image.write_to(&mut out, image::ImageFormat::WebP)?;
            Ok(out.into_inner())
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        TRACK_THUMBNAILS.lock().await.insert(name, bytes.clone());
        bytes
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response())
}

async fn track_cover(
    State(state): State<Arc<Dashboard>>,
    RoutePath(name): RoutePath<String>,
) -> WebResult<Response> {
    let path = state
        .bot
        .catalog
        .stations
        .iter()
        .flat_map(|s| &s.tracks)
        .filter_map(|t| t.cover.as_ref())
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(name.as_str()))
        .ok_or(WebError(StatusCode::NOT_FOUND, "Cover not found."))?;
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| WebError(StatusCode::NOT_FOUND, "Cover not found."))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response())
}

async fn dashboard_page(Query(params): Query<HashMap<String, String>>) -> Html<String> {
    dashboard_html(
        params.get("server").is_some_and(|id| valid_server_path(id)),
        &params,
    )
}

async fn server_dashboard_page(
    RoutePath(id): RoutePath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, StatusCode> {
    if !valid_server_path(&id) {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(dashboard_html(true, &params))
}

fn valid_server_path(id: &str) -> bool {
    id.as_bytes()
        .first()
        .is_some_and(|digit| matches!(digit, b'1'..=b'9'))
        && id.parse::<u64>().is_ok()
}

fn dashboard_html(workspace: bool, params: &HashMap<String, String>) -> Html<String> {
    let html = include_str!("../web/dashboard.html");
    if workspace {
        let view = match params.get("page").map(String::as_str) {
            Some("stations") if params.get("album").is_some_and(|album| !album.is_empty()) => {
                "album"
            }
            Some("stations") => "stations",
            Some("settings" | "playback" | "access" | "community") => "settings",
            Some("profile") => "profile",
            _ => "overview",
        };
        Html(
            html.replace("<aside id=\"sidebar\" hidden", "<aside id=\"sidebar\"")
                .replace("<section id=\"loading\"", "<section id=\"loading\" hidden")
                .replace(
                    "data-view=\"overview\" role=\"status\" hidden",
                    &format!("data-view=\"{view}\" role=\"status\""),
                ),
        )
    } else {
        Html(html.to_owned())
    }
}

fn router(state: Arc<Dashboard>) -> Router {
    Router::new()
        .route("/music/covers/{name}", get(track_cover))
        .route("/music/albums/{name}", get(album_cover))
        .route("/music/thumbnails/{name}", get(track_thumbnail))
        .route(
            "/music/navigation.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/navigation.js"),
                )
            }),
        )
        .route("/music/api/stations", get(station_catalog))
        .route(
            "/music/api/guilds/{guild}/track-listening",
            get(track_listening),
        )
        .route("/music/api/recordings/{id}/audio", get(recording_preview))
        .route(
            "/music/api/guilds/{guild}/playback",
            axum::routing::put(playback_timer).route_layer(middleware::from_fn_with_state(
                state.clone(),
                login_before_body,
            )),
        )
        .route(
            "/music/api/guilds/{guild}/server-settings",
            get(expanded_settings)
                .put(save_expanded)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    login_before_body,
                )),
        )
        .route(
            "/music/api/guilds/{guild}/personal",
            get(personal)
                .put(save_personal)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    login_before_body,
                )),
        )
        .route("/music/api/guilds/{guild}/community", get(community))
        .route(
            "/music/api/guilds/{guild}/profile",
            get(bot_profile)
                .put(save_bot_profile)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    login_before_body,
                ))
                .layer(DefaultBodyLimit::max(6 * 1024 * 1024)),
        )
        .route("/music", get(dashboard_page))
        .route("/music/", get(dashboard_page))
        .route("/music/dashboard", get(dashboard_page))
        .route("/music/{server}", get(server_dashboard_page))
        .route(
            "/music/dashboard.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/dashboard.css"),
                )
            }),
        )
        .route(
            "/music/assets/fonts/{font}",
            get(|RoutePath(font): RoutePath<String>| async move {
                let bytes = match font.as_str() {
                    "unbounded-500-latin.woff2" => {
                        include_bytes!("../assets/fonts/unbounded-500-latin.woff2").as_slice()
                    }
                    "unbounded-500-latin-ext.woff2" => {
                        include_bytes!("../assets/fonts/unbounded-500-latin-ext.woff2").as_slice()
                    }
                    _ => return StatusCode::NOT_FOUND.into_response(),
                };
                ([(header::CONTENT_TYPE, "font/woff2")], bytes).into_response()
            }),
        )
        .route(
            "/music/server-picker.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/server-picker.css"),
                )
            }),
        )
        .route(
            "/music/dashboard-i18n.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/dashboard-i18n.js"),
                )
            }),
        )
        .route(
            "/music/dashboard.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/dashboard.js"),
                )
            }),
        )
        .route(
            "/music/avatar.png",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    include_bytes!("../assets/brand/v4/vozen-music-avatar.png").as_slice(),
                )
            }),
        )
        .route(
            "/music/vozen.svg",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/svg+xml")],
                    include_str!("../web/vozen.svg"),
                )
            }),
        )
        .route(
            "/music/assets/flags-v2/{flag}",
            get(|RoutePath(flag): RoutePath<String>| async move {
                let svg = match flag.as_str() {
                    "gb.svg" => include_str!("../web/flags/gb.svg"),
                    "pt.svg" => include_str!("../web/flags/pt.svg"),
                    "fr.svg" => include_str!("../web/flags/fr.svg"),
                    "es.svg" => include_str!("../web/flags/es.svg"),
                    "de.svg" => include_str!("../web/flags/de.svg"),
                    "tr.svg" => include_str!("../web/flags/tr.svg"),
                    "sa.svg" => include_str!("../web/flags/sa.svg"),
                    "tw.svg" => include_str!("../web/flags/tw.svg"),
                    "ru.svg" => include_str!("../web/flags/ru.svg"),
                    "kr.svg" => include_str!("../web/flags/kr.svg"),
                    _ => return StatusCode::NOT_FOUND.into_response(),
                };
                ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
            }),
        )
        .route(
            "/music/css/global-nav-v1.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/global-nav-v1.css"),
                )
            }),
        )
        .route(
            "/music/assets/fonts/outfit-400-latin.woff2",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/woff2")],
                    include_bytes!("../web/outfit.woff2").as_slice(),
                )
            }),
        )
        .route(
            "/music/assets/fonts/unbounded-brand-latin.woff2",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/woff2")],
                    include_bytes!("../web/header-font.woff2").as_slice(),
                )
            }),
        )
        .route(
            "/music/outfit.woff2",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/woff2")],
                    include_bytes!("../web/outfit.woff2").as_slice(),
                )
            }),
        )
        .route("/music/auth/login", get(authorize))
        .route("/music/auth/callback", get(callback))
        .route("/music/auth/logout", post(logout))
        .route(
            "/music/auth/vozen",
            post(account_bridge).options(account_preflight),
        )
        .route(
            "/music/auth/vozen/logout",
            post(account_logout).options(account_preflight),
        )
        .route("/music/api/me", get(me))
        .route("/music/api/guilds", get(guilds))
        .route(
            "/music/api/guilds/{guild}/settings",
            get(settings)
                .put(save)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    login_before_body,
                )),
        )
        .route(
            "/music/health",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .layer(DefaultBodyLimit::max(8192))
        .layer(middleware::from_fn_with_state(state.clone(), guarded))
        .with_state(state)
}

pub async fn start(
    bot: Arc<Bot>,
    cache: Arc<Cache>,
    http: Arc<Http>,
) -> Result<Option<tokio::task::JoinHandle<Result<(), Error>>>, Error> {
    let Some(origin) = std::env::var("MUSIC_DASHBOARD_ORIGIN")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if !origin_valid(&origin) {
        return Err(
            "MUSIC_DASHBOARD_ORIGIN must be an HTTPS origin without a trailing slash or path."
                .into(),
        );
    }
    let secret = std::env::var("DISCORD_CLIENT_SECRET")
        .map_err(|_| "Missing DISCORD_CLIENT_SECRET for the dashboard.")?;
    if secret.trim().len() < 16 {
        return Err("Invalid DISCORD_CLIENT_SECRET.".into());
    }
    let state = Arc::new(Dashboard {
        bot,
        cache,
        http,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        origin,
        secret,
        client_id: std::env::var("DISCORD_APPLICATION_ID")?,
        auth: Mutex::new(AuthStore::default()),
    });
    let bind = std::env::var("MUSIC_HTTP_BIND").unwrap_or_else(|_| "0.0.0.0:8790".to_owned());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("Vozen Music dashboard HTTP listener ready.");
    Ok(Some(tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(Into::into)
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn account_token_requires_the_expected_application_scopes_and_expiry() {
        let expires = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
        let mut oauth = serde_json::json!({"application":{"id":ACCOUNT_CLIENT_ID},"scopes":["identify","guilds"],"expires":expires});
        assert!(account_authorized(&oauth).is_some());
        oauth["application"]["id"] = "other-app".into();
        assert!(account_authorized(&oauth).is_none());
        oauth["application"]["id"] = ACCOUNT_CLIENT_ID.into();
        oauth["scopes"] = serde_json::json!(["identify"]);
        assert!(account_authorized(&oauth).is_none());
        oauth["scopes"] = serde_json::json!(["identify", "guilds"]);
        oauth["expires"] = "2020-01-01T00:00:00Z".into();
        assert!(account_authorized(&oauth).is_none());
    }

    #[tokio::test]
    async fn account_bridge_cors_and_logout_are_scoped_to_the_vozen_origin() {
        use axum::{body::Body, http::Request};
        let state = fixture();
        let app = router(state.clone());
        let path = "/music/auth/vozen/logout";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method("OPTIONS")
                    .header("origin", ACCOUNT_ORIGIN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            ACCOUNT_ORIGIN
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method("POST")
                    .header("origin", "https://evil.example")
                    .header("content-type", "application/json")
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            state
                .auth
                .lock()
                .await
                .sessions
                .contains_key("test-session")
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method("POST")
                    .header("origin", ACCOUNT_ORIGIN)
                    .header("content-type", "application/json")
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            !state
                .auth
                .lock()
                .await
                .sessions
                .contains_key("test-session")
        );
        assert!(
            response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
    }

    fn test_login(user: &str, expires: Instant) -> Login {
        Login::new(
            UserInfo {
                id: user.into(),
                username: "Test manager".into(),
                global_name: None,
                avatar: None,
                avatar_decoration_data: None,
            },
            "synthetic-token".into(),
            expires,
            true,
        )
    }

    pub(super) fn fixture() -> Arc<Dashboard> {
        let mut auth = AuthStore::default();
        let mut login = test_login("123", Instant::now() + SESSION_TTL);
        login.token = "never-expose-this-token".into();
        login.csrf = "test-csrf".into();
        auth.sessions.insert("test-session".into(), login);
        Arc::new(Dashboard {
            bot: Arc::new(Bot {
                catalog: Arc::new(Catalog { stations: vec![] }),
                guild: None,
                entitlements: None,
                playback_queue: Arc::default(),
                background: Arc::new(AtomicBool::new(false)),
                sessions: Arc::new(Mutex::new(HashMap::new())),
                settings: Arc::new(Mutex::new(
                    SettingsStore::load(
                        std::env::temp_dir().join(format!("music-web-{}.json", nonce())),
                    )
                    .unwrap(),
                )),
                play_cooldowns: Arc::default(),
            }),
            cache: Arc::new(Cache::default()),
            http: Arc::new(Http::new("synthetic-token")),
            client: reqwest::Client::new(),
            origin: "https://api.vozen.org".into(),
            client_id: "1564573935234973817".into(),
            secret: "synthetic-client-secret".into(),
            auth: Mutex::new(auth),
        })
    }

    #[tokio::test]
    async fn initial_loading_matches_the_server_route_before_scripts_run() {
        use axum::{body::Body, http::Request};
        let app = router(fixture());
        for (path, view) in [
            ("/music", None),
            ("/music/1", Some("overview")),
            ("/music/1?page=playback", Some("settings")),
            ("/music/1?page=stations", Some("stations")),
            ("/music/1?page=profile", Some("profile")),
            ("/music/1?page=stations&album=rain", Some("album")),
            (
                "/music/dashboard?server=1&page=stations&album=rain",
                Some("album"),
            ),
            ("/music/dashboard?server=invalid", None),
            ("/music/1?page=%22%3E%3Cscript%3E", Some("overview")),
        ] {
            let workspace = view.is_some();
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let text = String::from_utf8(
                axum::body::to_bytes(response.into_body(), 100_000)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert_eq!(text.contains("<section id=\"loading\" hidden"), workspace);
            assert_eq!(text.contains("<aside id=\"sidebar\" hidden"), !workspace);
            let state = match view {
                Some(view) => format!("data-view=\"{view}\" role=\"status\">"),
                None => "data-view=\"overview\" role=\"status\" hidden>".to_owned(),
            };
            assert!(text.contains(&state), "wrong skeleton for {path}");
            assert!(!text.contains("<script>"));
        }
    }

    #[tokio::test]
    async fn browser_preview_is_private_and_streams_only_catalog_recordings() {
        use axum::{body::Body, http::Request};
        let mut state = fixture();
        let catalog = Catalog::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let track = &catalog.stations[0].tracks[0];
        let uri = format!("/music/api/recordings/{}/audio", track.id);
        let audio = std::fs::read(&track.file).unwrap();
        Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().bot)
            .unwrap()
            .catalog = Arc::new(catalog);
        let app = router(state.clone());
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        for (range, expected) in [
            (None, &audio[..]),
            (Some("bytes=0-63"), &audio[..64]),
            (Some("bytes=64-"), &audio[64..]),
            (Some("bytes=-32"), &audio[audio.len() - 32..]),
        ] {
            let mut request = Request::builder()
                .uri(&uri)
                .header("cookie", "__Secure-vozen_music=test-session");
            if let Some(range) = range {
                request = request.header(header::RANGE, range);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if range.is_some() {
                    StatusCode::PARTIAL_CONTENT
                } else {
                    StatusCode::OK
                }
            );
            assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/ogg");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "private, no-store"
            );
            assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
            assert_eq!(
                response.headers()[header::CONTENT_LENGTH],
                expected.len().to_string()
            );
            let bytes = axum::body::to_bytes(response.into_body(), audio.len())
                .await
                .unwrap();
            assert_eq!(bytes.as_ref(), expected);
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri(&uri)
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            audio.len().to_string()
        );
        assert!(
            axum::body::to_bytes(response.into_body(), 1)
                .await
                .unwrap()
                .is_empty()
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .header(header::RANGE, "bytes=999999999999-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response.headers()[header::CONTENT_RANGE],
            format!("bytes */{}", audio.len())
        );
        for suffix in ["unknown", "catalog.json", ".env"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/music/api/recordings/{suffix}/audio"))
                        .header("cookie", "__Secure-vozen_music=test-session")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        assert!(
            state.bot.sessions.lock().await.is_empty(),
            "Browser previews cannot create Discord sessions"
        );
    }

    #[test]
    fn browser_preview_ranges_are_bounded() {
        for (input, expected) in [
            (None, Some((0, 99))),
            (Some("bytes=0-0"), Some((0, 0))),
            (Some("bytes=10-"), Some((10, 99))),
            (Some("bytes=10-999"), Some((10, 99))),
            (Some("bytes=-20"), Some((80, 99))),
            (Some("bytes=-999"), Some((0, 99))),
        ] {
            assert_eq!(preview_range(input, 100), expected);
        }
        for value in [
            "bytes=100-",
            "bytes=10-1",
            "bytes=-0",
            "bytes=",
            "bytes=0-1,3-4",
            "bytes=+1-2",
            "bytes= 1-2",
            "items=0-1",
            "bytes=18446744073709551616-",
        ] {
            assert_eq!(preview_range(Some(value), 100), None, "{value}");
        }
        assert_eq!(preview_range(None, 0), None);
    }

    #[tokio::test]
    async fn album_library_artwork_durations_and_thumbnails_match_the_catalog() {
        let mut state = fixture();
        let catalog = Catalog::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let expected_tracks: std::collections::BTreeMap<_, _> = catalog
            .stations
            .iter()
            .map(|s| (s.id.clone(), s.tracks.len()))
            .collect();
        let names: Vec<_> = catalog
            .stations
            .iter()
            .map(|s| s.id.clone())
            .chain(std::iter::once("my".into()))
            .collect();
        let first_cover = catalog.stations[0].tracks[0]
            .cover
            .as_ref()
            .unwrap()
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().bot)
            .unwrap()
            .catalog = Arc::new(catalog);
        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/music/api/stations")
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 512 * 1024)
            .await
            .unwrap();
        let library: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let stations = library["stations"].as_array().unwrap();
        assert_eq!(stations.len(), expected_tracks.len());
        let proposals = library["proposals"].as_array().unwrap();
        let mut proposed_ids = std::collections::BTreeSet::new();
        for proposal in proposals {
            let id = proposal["id"].as_str().unwrap();
            assert!(proposed_ids.insert(id));
            assert!(!stations.iter().any(|s| s["id"] == id));
            assert!(proposal.get("tracks").is_none());
            assert_eq!(proposal["cover"], format!("/music/albums/{id}-v1.webp"));
        }
        for station in stations {
            let id = station["id"].as_str().unwrap();
            assert_eq!(
                station["tracks"].as_array().unwrap().len(),
                expected_tracks[id]
            );
            for track in station["tracks"].as_array().unwrap() {
                assert!(track["duration_seconds"].as_f64().unwrap() > 0.0);
                if let Some(thumbnail) = track["thumbnail"].as_str() {
                    assert!(thumbnail.starts_with("/music/thumbnails/"));
                }
                assert_eq!(track["license"], "CC0-1.0");
            }
        }
        for name in names {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/music/albums/{name}-v1.webp"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
            let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
                .await
                .unwrap();
            let picture = image::load_from_memory(&bytes).unwrap();
            assert_eq!((picture.width(), picture.height()), (640, 640));
        }
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/music/thumbnails/{first_cover}.webp"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
            let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
                .await
                .unwrap();
            let picture = image::load_from_memory(&bytes).unwrap();
            assert!(picture.width() <= 128 && picture.height() <= 128);
        }
        for path in [
            "/music/albums/unknown-v1.webp",
            "/music/albums/.env",
            "/music/thumbnails/unknown.webp",
            "/music/thumbnails/.env",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn artwork_is_public_but_only_catalog_covers_are_served() {
        let mut state = fixture();
        let catalog = Catalog::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let cover = catalog.stations[0].tracks[0]
            .cover
            .as_ref()
            .unwrap()
            .clone();
        let name = cover.file_name().unwrap().to_str().unwrap();
        Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().bot)
            .unwrap()
            .catalog = Arc::new(catalog);
        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/music/covers/{name}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), std::fs::read(cover).unwrap());
        for path in [
            "/music/covers/catalog.json",
            "/music/covers/.env",
            "/music/covers/unknown.png",
            "/music/covers/%2e%2e%2fcatalog.json",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn settings_routes_require_login_csrf_and_installed_server() {
        use axum::{body::Body, http::Request};
        let app = router(fixture());
        let path = "/music/api/guilds/1524199248248176751/settings";
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/music/api/guilds/1524199248248176751/track-listening")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let payload = r#"{"access":"everyone","mode":"instant","majority_scope":"call","moderator_roles":[]}"#;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .header("origin", "https://api.vozen.org")
                    .header("x-csrf-token", "test-csrf")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn picker_fonts_stay_inside_the_public_music_prefix() {
        let app = router(fixture());
        for (font, status) in [
            ("unbounded-500-latin.woff2", StatusCode::OK),
            ("unbounded-500-latin-ext.woff2", StatusCode::OK),
            ("not-a-public-font.woff2", StatusCode::NOT_FOUND),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/music/assets/fonts/{font}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            if status == StatusCode::OK {
                assert_eq!(response.headers()["content-type"], "font/woff2");
            }
        }
        assert!(include_str!("../web/server-picker.css").contains("url('assets/fonts/"));
    }

    #[tokio::test]
    async fn responses_keep_tokens_private_and_issue_secure_oauth_cookies() {
        use axum::{
            body::{Body, to_bytes},
            http::Request,
        };
        let app = router(fixture());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/music/api/me")
                    .header("cookie", "__Secure-vozen_music=test-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("test-csrf"));
        assert!(!body.contains("never-expose"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/music/auth/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location =
            reqwest::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        assert_eq!(location.host_str(), Some("discord.com"));
        assert!(
            location
                .query_pairs()
                .any(|(name, value)| name == "scope" && value == "identify guilds")
        );
        let cookie = response.headers()["set-cookie"].to_str().unwrap();
        for flag in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/music"] {
            assert!(cookie.contains(flag));
        }
    }

    #[test]
    fn policy_rejects_foreign_deleted_and_everyone_roles() {
        let roles = [(RoleId::new(200), Role::default())].into();
        let make = |id: &str| Policy {
            access: vozen_music::skip::SkipAccess::Moderators,
            mode: vozen_music::skip::SkipMode::Majority,
            majority_scope: vozen_music::skip::MajorityScope::Eligible,
            moderator_roles: vec![id.to_owned()],
        };
        assert!(validate_policy(make("200"), GuildId::new(100), &roles).is_ok());
        assert!(validate_policy(make("100"), GuildId::new(100), &roles).is_err());
        assert!(validate_policy(make("300"), GuildId::new(100), &roles).is_err());
    }

    #[test]
    fn oauth_state_is_browser_bound_single_use_and_expires() {
        let now = Instant::now();
        let mut auth = AuthStore::default();
        let client = ClientIp::new(IpAddr::from([192, 0, 2, 1]));
        auth.insert_pending("state".into(), client, now);
        assert!(!auth.consume_state("state", Some("other"), now));
        assert!(auth.consume_state("state", Some("state"), now));
        assert!(!auth.consume_state("state", Some("state"), now));
        auth.pending.insert(
            "old".into(),
            Pending {
                expires: now,
                client,
            },
        );
        assert!(!auth.consume_state("old", Some("old"), now));
    }

    #[test]
    fn client_address_comes_from_the_local_proxy_or_the_socket_peer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.9, 203.0.113.5".parse().unwrap(),
        );
        let loopback_ip = IpAddr::from([127, 0, 0, 1]);
        let public_ip = IpAddr::from([198, 51, 100, 77]);
        let loopback = Some(loopback_ip);
        let bridge = Some(IpAddr::from([172, 17, 0, 1]));
        let public = Some(public_ip);
        // Right-most X-Forwarded-For entry is the one the local proxy appended.
        assert_eq!(
            client_ip(&headers, loopback),
            ClientIp(IpAddr::from([203, 0, 113, 5]))
        );
        headers.insert("cf-connecting-ip", "192.0.2.44".parse().unwrap());
        assert_eq!(
            client_ip(&headers, bridge),
            ClientIp(IpAddr::from([192, 0, 2, 44]))
        );
        // A directly connected public peer cannot choose its own bucket.
        assert_eq!(client_ip(&headers, public), ClientIp(public_ip));
        headers.insert("cf-connecting-ip", "not-an-ip".parse().unwrap());
        headers.insert("x-forwarded-for", "garbage".parse().unwrap());
        assert_eq!(client_ip(&headers, loopback), ClientIp(loopback_ip));
        // IPv6 clients share one bucket per /64.
        let mut v6 = HeaderMap::new();
        v6.insert("cf-connecting-ip", "2001:db8:1:2:aaaa::1".parse().unwrap());
        let mut other = HeaderMap::new();
        other.insert("cf-connecting-ip", "2001:db8:1:2:bbbb::9".parse().unwrap());
        assert_eq!(client_ip(&v6, loopback), client_ip(&other, loopback));
    }

    #[test]
    fn one_client_cannot_exhaust_the_limit_for_everyone_or_the_oauth_callback() {
        let now = Instant::now();
        let mut limits = RequestLimits::default();
        let noisy = ClientIp::new(IpAddr::from([203, 0, 113, 1]));
        let quiet = ClientIp::new(IpAddr::from([203, 0, 113, 2]));
        for _ in 0..CLIENT_REQUESTS {
            assert!(limits.check(noisy, true, now));
        }
        assert!(!limits.check(noisy, true, now));
        assert!(limits.check(quiet, true, now));
        assert!(limits.check(noisy, true, now + RATE_WINDOW));
        // When the global ceiling is reached, the OAuth callback still works.
        let mut limits = RequestLimits::default();
        for index in 0..GLOBAL_REQUESTS {
            let client = ClientIp::new(IpAddr::from((index as u32 + 1).to_be_bytes()));
            assert!(limits.check(client, true, now));
        }
        assert!(!limits.check(quiet, true, now));
        assert!(limits.check(quiet, false, now));
        // The tracked-client map stays bounded and evicts instead of refusing.
        assert!(limits.clients.len() <= MAX_TRACKED_CLIENTS);
        let mut limits = RequestLimits::default();
        for index in 0..MAX_TRACKED_CLIENTS + 10 {
            let client = ClientIp::new(IpAddr::from((index as u32 + 1).to_be_bytes()));
            assert!(limits.check(client, false, now));
        }
        assert_eq!(limits.clients.len(), MAX_TRACKED_CLIENTS);
        assert!(rate_exempt("/music/assets/fonts/outfit-400-latin.woff2"));
        assert!(rate_exempt("/music/covers/a.png"));
        assert!(!rate_exempt("/music/api/me"));
        assert!(!rate_exempt("/music/auth/callback"));
    }

    #[tokio::test]
    async fn rate_limit_applies_per_forwarded_client_through_the_router() {
        use axum::{body::Body, http::Request};
        let app = router(fixture());
        let request = |ip: &str, path: &str| {
            Request::builder()
                .uri(path)
                .header("cf-connecting-ip", ip)
                .body(Body::empty())
                .unwrap()
        };
        for _ in 0..CLIENT_REQUESTS {
            let response = app
                .clone()
                .oneshot(request("203.0.113.7", "/music/api/me"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let blocked = app
            .clone()
            .oneshot(request("203.0.113.7", "/music/api/me"))
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
        let asset = app
            .clone()
            .oneshot(request("203.0.113.7", "/music/dashboard.css"))
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        let other = app
            .oneshot(request("203.0.113.8", "/music/api/me"))
            .await
            .unwrap();
        assert_eq!(other.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn pending_logins_are_capped_per_client_and_evict_the_oldest() {
        let now = Instant::now();
        let mut auth = AuthStore::default();
        let attacker = ClientIp::new(IpAddr::from([203, 0, 113, 1]));
        for index in 0..MAX_PENDING_PER_CLIENT + 3 {
            auth.insert_pending(
                format!("a{index}"),
                attacker,
                now + Duration::from_millis(index as u64),
            );
        }
        assert_eq!(auth.pending.len(), MAX_PENDING_PER_CLIENT);
        assert!(!auth.pending.contains_key("a0"));
        assert!(
            auth.pending
                .contains_key(&format!("a{}", MAX_PENDING_PER_CLIENT + 2))
        );
        // A full store evicts the oldest state rather than refusing new sign-ins.
        let mut auth = AuthStore::default();
        for index in 0..MAX_PENDING + 1 {
            let client = ClientIp::new(IpAddr::from((index as u32 + 1).to_be_bytes()));
            auth.insert_pending(
                format!("s{index}"),
                client,
                now + Duration::from_millis(index as u64),
            );
        }
        assert_eq!(auth.pending.len(), MAX_PENDING);
        assert!(!auth.pending.contains_key("s0"));
        assert!(auth.pending.contains_key(&format!("s{MAX_PENDING}")));
    }

    #[test]
    fn sessions_are_capped_per_user_and_full_store_evicts_instead_of_refusing() {
        let now = Instant::now();
        let mut auth = AuthStore::default();
        auth.insert_session(
            "other".into(),
            test_login("2", now + SESSION_TTL),
            None,
            now,
        );
        for index in 0..MAX_SESSIONS_PER_USER + 2 {
            auth.insert_session(
                format!("u{index}"),
                test_login("1", now + SESSION_TTL + Duration::from_secs(index as u64)),
                None,
                now,
            );
        }
        let own = auth.sessions.values().filter(|l| l.user.id == "1").count();
        assert_eq!(own, MAX_SESSIONS_PER_USER);
        assert!(!auth.sessions.contains_key("u0") && !auth.sessions.contains_key("u1"));
        assert!(auth.sessions.contains_key("other"));
        // Signing in again replaces the browser's previous session.
        auth.insert_session(
            "fresh".into(),
            test_login("2", now + SESSION_TTL),
            Some("other"),
            now,
        );
        assert!(!auth.sessions.contains_key("other"));
        let mut auth = AuthStore::default();
        for index in 0..MAX_SESSIONS + 1 {
            auth.insert_session(
                format!("g{index}"),
                test_login(
                    &(index + 1).to_string(),
                    now + SESSION_TTL + Duration::from_secs(index as u64),
                ),
                None,
                now,
            );
        }
        assert_eq!(auth.sessions.len(), MAX_SESSIONS);
        assert!(!auth.sessions.contains_key("g0"));
        assert!(auth.sessions.contains_key(&format!("g{MAX_SESSIONS}")));
    }

    #[test]
    fn manager_channels_respect_view_permission_overwrites() {
        let guild_id = GuildId::new(100);
        let mut guild = Guild::default();
        guild.id = guild_id;
        guild.owner_id = UserId::new(1);
        let mut everyone = Role::default();
        everyone.id = RoleId::new(100);
        everyone.permissions = Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD;
        let mut staff = Role::default();
        staff.id = RoleId::new(200);
        guild.roles = [(everyone.id, everyone), (staff.id, staff)].into();
        let mut member = Member::default();
        member.user.id = UserId::new(5);
        member.guild_id = guild_id;
        let overwrite = |allow, deny, role| PermissionOverwrite {
            allow,
            deny,
            kind: PermissionOverwriteType::Role(RoleId::new(role)),
        };
        let hidden = overwrite(Permissions::empty(), Permissions::VIEW_CHANNEL, 100);
        let staff_only = overwrite(Permissions::VIEW_CHANNEL, Permissions::empty(), 200);
        let channel = |id, kind, overwrites: Vec<PermissionOverwrite>| {
            let mut c = GuildChannel::default();
            c.id = ChannelId::new(id);
            c.guild_id = guild_id;
            c.kind = kind;
            c.permission_overwrites = overwrites;
            c
        };
        let channels = [
            channel(1, ChannelType::Text, vec![]),
            channel(2, ChannelType::Voice, vec![hidden.clone()]),
            channel(3, ChannelType::Text, vec![hidden, staff_only]),
            channel(4, ChannelType::Category, vec![]),
        ];
        let ids = |member: &Member| {
            visible_channels(&channels, member, |c, m| guild.user_permissions_in(c, m))
                .iter()
                .map(|c| c.id.get())
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(ids(&member), [1].into());
        member.roles = vec![RoleId::new(200)];
        assert_eq!(ids(&member), [1, 3].into());
        member.user.id = UserId::new(1);
        assert_eq!(ids(&member), [1, 2, 3].into());
        // Saving: new hidden choices are refused, unchanged earlier choices are kept.
        let visible = [1, 3].into();
        let stored = ServerSettings::default();
        let mut chosen = ServerSettings {
            announcement_channel: Some(3),
            ..ServerSettings::default()
        };
        assert!(chosen_channels_visible(&chosen, &stored, &visible));
        chosen.voice_channel = Some(2);
        assert!(!chosen_channels_visible(&chosen, &stored, &visible));
        let stored = ServerSettings {
            voice_channel: Some(2),
            ..ServerSettings::default()
        };
        assert!(chosen_channels_visible(&chosen, &stored, &visible));
        chosen.announcement_channel = Some(9);
        assert!(!chosen_channels_visible(&chosen, &stored, &visible));
    }

    #[tokio::test]
    async fn audio_previews_are_limited_per_session() {
        use axum::{body::Body, http::Request};
        let mut state = fixture();
        // A synthetic catalog over a tracked recording; covers are not needed.
        let catalog: Catalog = serde_json::from_value(serde_json::json!({"stations":[{
            "id":"lofi","name":"Lofi","tracks":[{
                "id":"preview-test","title":"t","artist":"a",
                "file":Path::new(env!("CARGO_MANIFEST_DIR")).join("media/lofi-again.ogg"),
                "source_url":"","license":"","license_evidence":"",
                "instrumental":true,"instrumental_evidence":"","approved":true}]}]}))
        .unwrap();
        let uri = "/music/api/recordings/preview-test/audio";
        Arc::get_mut(&mut Arc::get_mut(&mut state).unwrap().bot)
            .unwrap()
            .catalog = Arc::new(catalog);
        let app = router(state);
        let request = || {
            Request::builder()
                .uri(uri)
                .header("cookie", "__Secure-vozen_music=test-session")
                .body(Body::empty())
                .unwrap()
        };
        let mut open = Vec::new();
        for _ in 0..PREVIEWS_PER_SESSION {
            let response = app.clone().oneshot(request()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            open.push(response);
        }
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        // Dropping an unfinished stream releases its slot.
        drop(open.pop());
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn csrf_needs_both_exact_origin_and_session_token() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("https://api.vozen.org"));
        headers.insert("x-csrf-token", HeaderValue::from_static("session-token"));
        assert!(csrf_valid(
            &headers,
            "https://api.vozen.org",
            "session-token"
        ));
        assert!(!csrf_valid(
            &headers,
            "https://evil.example",
            "session-token"
        ));
        assert!(!csrf_valid(&headers, "https://api.vozen.org", "other"));
        headers.remove("origin");
        assert!(!csrf_valid(
            &headers,
            "https://api.vozen.org",
            "session-token"
        ));
    }

    #[test]
    fn permissions_do_not_grant_dashboard_to_regular_moderators() {
        assert!(can_manage(true, "0"));
        assert!(can_manage(false, "8"));
        assert!(can_manage(false, "32"));
        assert!(!can_manage(false, "8192"));
        assert!(!can_manage(false, "invalid"));
    }

    #[test]
    fn snowflakes_round_trip_without_javascript_number_precision_loss() {
        let original = SkipSettings {
            moderator_roles: [1524199248248176751].into(),
            ..Default::default()
        };
        let json = serde_json::to_value(Policy::from(original)).unwrap();
        assert_eq!(json["moderator_roles"][0], "1524199248248176751");
        assert!(parse_id("0").is_err());
        assert!(parse_id("1/roles").is_err());
        assert!(parse_id("18446744073709551616").is_err());
    }

    #[test]
    fn only_https_origins_are_configurable() {
        assert!(origin_valid("https://api.vozen.org"));
        for value in [
            "http://api.vozen.org",
            "https://api.vozen.org/",
            "https://a/b",
            "https://u:p@a",
            "https://a?q=x",
        ] {
            assert!(!origin_valid(value), "{value}");
        }
    }
}
