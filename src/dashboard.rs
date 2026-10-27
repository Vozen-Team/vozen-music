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
use std::{collections::VecDeque, time::Instant};

const API: &str = "https://discord.com/api/v10";
const SESSION_COOKIE: &str = "__Secure-vozen_music";
const STATE_COOKIE: &str = "__Secure-vozen_music_oauth";
const SESSION_TTL: Duration = Duration::from_secs(3600);
const STATE_TTL: Duration = Duration::from_secs(600);

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
}

#[derive(Clone, Serialize, Deserialize)]
struct UserInfo {
    id: String,
    username: String,
}

#[derive(Default)]
struct AuthStore {
    pending: HashMap<String, Instant>,
    sessions: HashMap<String, Login>,
    requests: VecDeque<Instant>,
}

impl AuthStore {
    fn prune(&mut self, now: Instant) {
        self.pending.retain(|_, expires| *expires > now);
        self.sessions.retain(|_, login| login.expires > now);
        while self
            .requests
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            self.requests.pop_front();
        }
    }

    fn consume_state(&mut self, value: &str, cookie: Option<&str>, now: Instant) -> bool {
        // A mismatched browser must not consume another browser's pending login.
        if cookie != Some(value) {
            return false;
        }
        self.pending
            .remove(value)
            .is_some_and(|expires| expires > now)
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
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let allowed = {
        let mut auth = state.auth.lock().await;
        auth.prune(Instant::now());
        if auth.requests.len() >= 600 {
            false
        } else {
            auth.requests.push_back(Instant::now());
            true
        }
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
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'",
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
        for role in member.roles {
            permissions |= server
                .roles
                .get(&role)
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
        Ok(server)
    }
}

async fn authorize(State(state): State<Arc<Dashboard>>) -> WebResult<Response> {
    let value = nonce();
    {
        let mut auth = state.auth.lock().await;
        auth.prune(Instant::now());
        if auth.pending.len() >= 200 || auth.sessions.len() >= 1000 {
            return Err(WebError(
                StatusCode::TOO_MANY_REQUESTS,
                "Sign-in is busy. Please try again later.",
            ));
        }
        auth.pending
            .insert(value.clone(), Instant::now() + STATE_TTL);
    }
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
        let mut response = Redirect::to("/music/dashboard?login=cancelled").into_response();
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
    {
        let mut auth = state.auth.lock().await;
        auth.prune(Instant::now());
        if auth.sessions.len() >= 1000 {
            return Err(WebError(
                StatusCode::TOO_MANY_REQUESTS,
                "Sign-in is busy. Try again later.",
            ));
        }
        if let Some(old) = cookie(&headers, SESSION_COOKIE) {
            auth.sessions.remove(old);
        }
        auth.sessions.insert(
            key.clone(),
            Login {
                user,
                token: token.access_token,
                csrf: nonce(),
                expires: Instant::now() + ttl,
            },
        );
    }
    let mut response = Redirect::to("/music/dashboard").into_response();
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
    let _ = state
        .client
        .post("https://discord.com/api/oauth2/token/revoke")
        .basic_auth(&state.client_id, Some(&state.secret))
        .form(&[("token", login.token)])
        .send()
        .await;
    let mut response = StatusCode::NO_CONTENT.into_response();
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
        let page: Vec<GuildInfo> = state
            .fetch(
                &format!("/users/@me/guilds?limit=200&after={after}"),
                &login.token,
            )
            .await?;
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
                .map(|guild| serde_json::json!({"id": guild.id, "name": guild.name})),
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
    let policy = Policy::from(state.bot.settings.lock().await.get(guild.get()));
    let mut roles: Vec<_> = server.roles.values().filter(|role| role.id.get() != guild.get())
        .map(|role| serde_json::json!({"id": role.id.to_string(), "name": role.name, "position": role.position})).collect();
    roles.sort_by_key(|role| std::cmp::Reverse(role["position"].as_i64().unwrap_or(0)));
    Ok(Json(
        serde_json::json!({"id": guild.to_string(), "name": server.name, "settings": policy, "roles": roles}),
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
    let session = {
        let sessions = state.bot.sessions.lock().await;
        state
            .bot
            .settings
            .lock()
            .await
            .set(guild.get(), policy.clone())
            .map_err(|_| {
                WebError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Settings could not be saved. Previous settings remain active. Try again.",
                )
            })?;
        if let Some(session) = sessions.get(&guild) {
            *session.policy.lock().map_err(|_| unavailable())? = policy.clone();
            session
                .votes
                .lock()
                .map_err(|_| unavailable())?
                .reset(session.generation.load(Ordering::SeqCst));
        }
        sessions.get(&guild).cloned()
    };
    if let Some(session) = session {
        player::refresh(&state.http, &state.bot.catalog, &session, false).await;
    }
    Ok(Json(serde_json::json!({"settings": Policy::from(policy)})))
}

fn router(state: Arc<Dashboard>) -> Router {
    Router::new()
        .route("/music", get(|| async { Redirect::to("/music/dashboard") }))
        .route(
            "/music/",
            get(|| async { Redirect::to("/music/dashboard") }),
        )
        .route(
            "/music/dashboard",
            get(|| async { Html(include_str!("../web/dashboard.html")) }),
        )
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
        .route("/music/api/me", get(me))
        .route("/music/api/guilds", get(guilds))
        .route(
            "/music/api/guilds/{guild}/settings",
            get(settings).put(save),
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
        axum::serve(listener, router(state))
            .await
            .map_err(Into::into)
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn fixture() -> Arc<Dashboard> {
        let mut auth = AuthStore::default();
        auth.sessions.insert(
            "test-session".into(),
            Login {
                user: UserInfo {
                    id: "123".into(),
                    username: "Test manager".into(),
                },
                token: "never-expose-this-token".into(),
                csrf: "test-csrf".into(),
                expires: Instant::now() + SESSION_TTL,
            },
        );
        Arc::new(Dashboard {
            bot: Arc::new(Bot {
                catalog: Arc::new(Catalog { stations: vec![] }),
                guild: None,
                sessions: Mutex::new(HashMap::new()),
                settings: Mutex::new(
                    SettingsStore::load(
                        std::env::temp_dir().join(format!("music-web-{}.json", nonce())),
                    )
                    .unwrap(),
                ),
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
        auth.pending.insert("state".into(), now + STATE_TTL);
        assert!(!auth.consume_state("state", Some("other"), now));
        assert!(auth.consume_state("state", Some("state"), now));
        assert!(!auth.consume_state("state", Some("state"), now));
        auth.pending.insert("old".into(), now);
        assert!(!auth.consume_state("old", Some("old"), now));
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
