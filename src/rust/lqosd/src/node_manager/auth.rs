//! Provides authentication for the Node Manager.

use crate::node_manager::runtime_onboarding::runtime_onboarding_state;
use crate::node_manager::security_headers::apply_node_manager_security_headers;
use axum::Json;
use axum::extract::ConnectInfo;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use lqos_config::{AuthenticatedUser, UserRole, WebUsers, load_config};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, warn};

const COOKIE_NAME: &str = "User-Token";
const SESSION_TOKEN_VERSION: &str = "v1";
const SESSION_DURATION_SECS: u64 = 60 * 60 * 24 * 30;
const SESSION_KEY_FILE_NAME: &str = "lqusers.session.key";
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const LOGIN_FAILURE_LIMIT: usize = 5;
const LOGIN_REPEATED_FAILURE_LOG_THRESHOLD: usize = 3;
const LOGIN_RATE_LIMIT_USERNAME_MAX_CHARS: usize = 128;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthFileFingerprint {
    path: PathBuf,
    modified_unix_secs: Option<u64>,
    modified_subsec_nanos: Option<u32>,
    len: u64,
}

#[derive(Clone, Debug)]
struct CachedAuthSnapshot {
    fingerprint: Option<AuthFileFingerprint>,
    snapshot: AuthSnapshot,
}

#[derive(Clone, Debug)]
struct AuthSnapshot {
    bootstrap_state: AuthBootstrapState,
    auth_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthBootstrapState {
    MissingUsersFile,
    NoUsersConfigured,
    Ready,
    CorruptUsersFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionClaims {
    sub: String,
    role: UserRole,
    auth_epoch: u64,
    iat: u64,
    exp: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LoginResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Clone, Debug)]
struct SessionUser {
    username: String,
    role: UserRole,
}

static AUTH_SNAPSHOT: Lazy<Mutex<Option<CachedAuthSnapshot>>> = Lazy::new(|| Mutex::new(None));
static SESSION_KEY: Lazy<Mutex<Option<Vec<u8>>>> = Lazy::new(|| Mutex::new(None));
static LOGIN_RATE_LIMITER: Lazy<Mutex<LoginRateLimiter>> =
    Lazy::new(|| Mutex::new(LoginRateLimiter::new()));
pub static FIRST_LOAD: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoginRateLimitScope {
    Ip,
    Username,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LoginRateLimitExceeded {
    scope: LoginRateLimitScope,
    failures: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LoginFailureRecord {
    ip_failures: usize,
    username_failures: usize,
}

#[derive(Debug)]
struct LoginRateLimiter {
    by_ip: HashMap<IpAddr, VecDeque<Instant>>,
    by_username: HashMap<String, VecDeque<Instant>>,
    last_cleanup: Instant,
}

impl LoginRateLimiter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            by_ip: HashMap::new(),
            by_username: HashMap::new(),
            last_cleanup: now,
        }
    }

    fn check(
        &mut self,
        remote_ip: IpAddr,
        username: &str,
        now: Instant,
    ) -> Option<LoginRateLimitExceeded> {
        self.prune_expired_entries(now);
        let ip_failures = Self::recent_failures(&mut self.by_ip, &remote_ip, now);
        if ip_failures >= LOGIN_FAILURE_LIMIT {
            return Some(LoginRateLimitExceeded {
                scope: LoginRateLimitScope::Ip,
                failures: ip_failures,
            });
        }

        let username_key = username.to_string();
        let username_failures = Self::recent_failures(&mut self.by_username, &username_key, now);
        if username_failures >= LOGIN_FAILURE_LIMIT {
            return Some(LoginRateLimitExceeded {
                scope: LoginRateLimitScope::Username,
                failures: username_failures,
            });
        }

        None
    }

    fn record_failure(
        &mut self,
        remote_ip: IpAddr,
        username: &str,
        now: Instant,
    ) -> LoginFailureRecord {
        self.prune_expired_entries(now);
        let ip_failures = Self::push_failure(&mut self.by_ip, remote_ip, now);
        let username_failures =
            Self::push_failure(&mut self.by_username, username.to_string(), now);

        LoginFailureRecord {
            ip_failures,
            username_failures,
        }
    }

    fn clear_username(&mut self, username: &str) {
        self.by_username.remove(username);
    }

    fn prune_expired_entries(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_cleanup) < LOGIN_FAILURE_WINDOW {
            return;
        }

        Self::prune_map(&mut self.by_ip, now);
        Self::prune_map(&mut self.by_username, now);
        self.last_cleanup = now;
    }

    fn prune_map<K: Eq + std::hash::Hash>(
        attempts_by_key: &mut HashMap<K, VecDeque<Instant>>,
        now: Instant,
    ) {
        attempts_by_key.retain(|_, attempts| {
            Self::prune_attempts(attempts, now);
            !attempts.is_empty()
        });
    }

    fn recent_failures<K: Eq + std::hash::Hash>(
        attempts_by_key: &mut HashMap<K, VecDeque<Instant>>,
        key: &K,
        now: Instant,
    ) -> usize {
        let Some(attempts) = attempts_by_key.get_mut(key) else {
            return 0;
        };
        Self::prune_attempts(attempts, now);
        attempts.len()
    }

    fn push_failure<K: Eq + std::hash::Hash>(
        attempts_by_key: &mut HashMap<K, VecDeque<Instant>>,
        key: K,
        now: Instant,
    ) -> usize {
        let attempts = attempts_by_key.entry(key).or_default();
        Self::prune_attempts(attempts, now);
        attempts.push_back(now);
        attempts.len()
    }

    fn prune_attempts(attempts: &mut VecDeque<Instant>, now: Instant) {
        while let Some(attempt) = attempts.front() {
            if now.saturating_duration_since(*attempt) <= LOGIN_FAILURE_WINDOW {
                break;
            }
            attempts.pop_front();
        }
    }
}

fn login_rate_limit_username(username: &str) -> String {
    let normalized: String = username
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .take(LOGIN_RATE_LIMIT_USERNAME_MAX_CHARS)
        .collect();

    if normalized.is_empty() {
        "<empty>".to_string()
    } else {
        normalized
    }
}

fn login_rate_limit_response() -> (StatusCode, Json<LoginResponse>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(LoginResponse {
            ok: false,
            reason: Some("rate_limited"),
            message: Some(
                "Too many failed login attempts. Wait a minute and try again.".to_string(),
            ),
        }),
    )
}

fn record_first_login_timestamp_if_needed() {
    let config = match load_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            warn!("Unable to load config to record first-login timestamp: {e}");
            return;
        }
    };

    let path = Path::new(&config.lqos_directory).join(".fl");
    if path.exists() {
        if FIRST_LOAD.load(Relaxed) != 0 {
            return;
        }
        let Ok(str) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(ts_int) = str.trim().parse::<u64>() else {
            return;
        };
        FIRST_LOAD.store(ts_int, Relaxed);
        return;
    }

    let ts = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(e) => {
            warn!("SystemTime before UNIX_EPOCH when recording first-login timestamp: {e:?}");
            return;
        }
    };

    if let Err(e) = std::fs::write(&path, ts.to_string()) {
        warn!("Failed to write first-login timestamp to {:?}: {e}", path);
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn auth_file_fingerprint(path: &Path) -> Result<AuthFileFingerprint, std::io::Error> {
    let metadata = std::fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok());

    Ok(AuthFileFingerprint {
        path: path.to_path_buf(),
        modified_unix_secs: modified.map(|d| d.as_secs()),
        modified_subsec_nanos: modified.map(|d| d.subsec_nanos()),
        len: metadata.len(),
    })
}

fn auth_snapshot() -> AuthSnapshot {
    let current_fingerprint = match WebUsers::existing_path() {
        Ok(Some(path)) => match auth_file_fingerprint(&path) {
            Ok(fingerprint) => Some(fingerprint),
            Err(e) => {
                warn!("Unable to stat auth file {:?}: {e}", path);
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            warn!("Unable to resolve auth file path: {e}");
            return AuthSnapshot {
                bootstrap_state: AuthBootstrapState::CorruptUsersFile,
                auth_epoch: 0,
            };
        }
    };

    let mut cache = AUTH_SNAPSHOT.lock();
    if let Some(cached) = &*cache
        && cached.fingerprint == current_fingerprint
    {
        return cached.snapshot.clone();
    }

    let snapshot = match current_fingerprint {
        None => AuthSnapshot {
            bootstrap_state: AuthBootstrapState::MissingUsersFile,
            auth_epoch: 0,
        },
        Some(_) => match WebUsers::load_or_create() {
            Ok(users) => AuthSnapshot {
                bootstrap_state: if users.is_empty() {
                    AuthBootstrapState::NoUsersConfigured
                } else {
                    AuthBootstrapState::Ready
                },
                auth_epoch: users.auth_epoch(),
            },
            Err(e) => {
                warn!("Unable to load auth state: {e}");
                AuthSnapshot {
                    bootstrap_state: AuthBootstrapState::CorruptUsersFile,
                    auth_epoch: 0,
                }
            }
        },
    };

    let refreshed_fingerprint = match WebUsers::existing_path() {
        Ok(Some(path)) => auth_file_fingerprint(&path).ok(),
        Ok(None) => None,
        Err(e) => {
            warn!("Unable to refresh auth file path after load: {e}");
            None
        }
    };

    *cache = Some(CachedAuthSnapshot {
        fingerprint: refreshed_fingerprint,
        snapshot: snapshot.clone(),
    });
    snapshot
}

fn session_key_path() -> Result<PathBuf, std::io::Error> {
    let config = load_config().map_err(|_| {
        std::io::Error::other("Unable to load /etc/lqos.conf while locating session key")
    })?;
    Ok(Path::new(&config.lqos_directory).join(SESSION_KEY_FILE_NAME))
}

fn session_key() -> Result<Vec<u8>, std::io::Error> {
    let mut cache = SESSION_KEY.lock();
    if let Some(key) = &*cache {
        return Ok(key.clone());
    }

    let path = session_key_path()?;
    let key = if path.exists() {
        let bytes = std::fs::read(&path)?;
        if bytes.is_empty() {
            return Err(std::io::Error::other(format!(
                "Session key file {:?} is empty",
                path
            )));
        }
        bytes
    } else {
        let mut new_key = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut new_key);

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(&new_key)?;
        drop(file);
        new_key
    };

    *cache = Some(key.clone());
    Ok(key)
}

fn build_session_cookie(token: String) -> Cookie<'static> {
    build_session_cookie_with_secure(token, session_cookie_secure())
}

fn build_session_cookie_with_secure(token: String, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new(COOKIE_NAME, token);
    cookie.set_path("/");
    cookie.set_same_site(SameSite::Lax);
    cookie.set_http_only(true);
    cookie.set_secure(secure);
    cookie
}

fn session_cookie_secure() -> bool {
    load_config()
        .ok()
        .and_then(|config| config.ssl.as_ref().map(|ssl| ssl.enabled))
        .unwrap_or(false)
}

fn build_signed_session(key: &[u8], user: &AuthenticatedUser) -> Result<String, StatusCode> {
    let now = now_unix_secs();
    let claims = SessionClaims {
        sub: user.username.clone(),
        role: user.role,
        auth_epoch: user.auth_epoch,
        iat: now,
        exp: now.saturating_add(SESSION_DURATION_SECS),
    };
    let payload = serde_json::to_vec(&claims).map_err(|e| {
        error!("Unable to serialize session claims: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload);

    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| {
        error!("Unable to initialize session signer: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    Ok(format!("{SESSION_TOKEN_VERSION}.{payload_b64}.{signature}"))
}

fn verify_signed_session(
    key: &[u8],
    token: &str,
    snapshot: &AuthSnapshot,
) -> Result<Option<SessionUser>, StatusCode> {
    let Some((version, remainder)) = token.split_once('.') else {
        return Ok(None);
    };
    if version != SESSION_TOKEN_VERSION {
        return Ok(None);
    }
    let Some((payload_b64, signature_b64)) = remainder.rsplit_once('.') else {
        return Ok(None);
    };

    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| {
        error!("Unable to initialize session verifier: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if mac.verify_slice(&signature).is_err() {
        return Ok(None);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let claims: SessionClaims =
        serde_json::from_slice(&payload).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let now = now_unix_secs();
    if claims.exp <= now || claims.auth_epoch != snapshot.auth_epoch {
        return Ok(None);
    }

    Ok(Some(SessionUser {
        username: claims.sub,
        role: claims.role,
    }))
}

fn session_from_cookie(
    jar: &CookieJar,
    snapshot: &AuthSnapshot,
) -> Result<Option<SessionUser>, StatusCode> {
    let Some(token) = jar.get(COOKIE_NAME) else {
        return Ok(None);
    };
    let key = session_key().map_err(|e| {
        error!("Unable to load session key: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    verify_signed_session(&key, token.value(), snapshot)
}

fn standalone_page_path(page: &str) -> Result<PathBuf, StatusCode> {
    let config = load_config().map_err(|e| {
        error!("Unable to load config for standalone page {page}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join(page))
}

fn serve_standalone_page(page: &str) -> Result<Response, StatusCode> {
    let path = standalone_page_path(page)?;
    let body = std::fs::read_to_string(&path).map_err(|e| {
        error!("Unable to read standalone page {:?}: {e}", path);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut response = Html(body).into_response();
    apply_node_manager_security_headers(response.headers_mut());
    Ok(response)
}

pub async fn login_page(jar: CookieJar) -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            Redirect::temporary("/first-run.html").into_response()
        }
        AuthBootstrapState::Ready => match session_from_cookie(&jar, &snapshot) {
            Ok(Some(_)) => Redirect::temporary(post_login_destination()).into_response(),
            Ok(None) => serve_standalone_page("login.html")
                .unwrap_or_else(|status| (status, "Unable to serve login page").into_response()),
            Err(status) => (status, "Unable to validate login session").into_response(),
        },
        AuthBootstrapState::CorruptUsersFile => serve_standalone_page("login.html")
            .unwrap_or_else(|status| (status, "Unable to serve login page").into_response()),
    }
}

pub async fn first_run_page() -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            serve_standalone_page("first-run.html")
                .unwrap_or_else(|status| (status, "Unable to serve first-run page").into_response())
        }
        AuthBootstrapState::Ready | AuthBootstrapState::CorruptUsersFile => {
            Redirect::temporary("/index.html").into_response()
        }
    }
}

pub async fn get_username(jar: &CookieJar) -> String {
    let snapshot = auth_snapshot();
    match session_from_cookie(jar, &snapshot) {
        Ok(Some(user)) => user.username,
        Ok(None) | Err(_) => "Anonymous".to_string(),
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LoginResult {
    Admin,
    ReadOnly,
    Denied,
}

fn login_result_for_session(user: Option<SessionUser>) -> LoginResult {
    match user {
        Some(SessionUser {
            role: UserRole::Admin,
            ..
        }) => LoginResult::Admin,
        Some(SessionUser {
            role: UserRole::ReadOnly,
            ..
        }) => LoginResult::ReadOnly,
        None => LoginResult::Denied,
    }
}

fn post_login_destination() -> &'static str {
    if runtime_onboarding_state().required {
        "/setup_runtime.html"
    } else {
        "/index.html"
    }
}

fn runtime_onboarding_exempt_path(path: &str) -> bool {
    matches!(
        path,
        "/setup_runtime.html"
            | "/config_integration.html"
            | "/config_splynx.html"
            | "/config_netzur.html"
            | "/config_visp.html"
            | "/config_uisp.html"
            | "/config_powercode.html"
            | "/config_sonar.html"
            | "/config_wispgate.html"
            | "/config_network.html"
            | "/config_devices.html"
            | "/help.html"
            | "/configuration.html"
    )
}

/// Checks an incoming request for a `User-Token` cookie. If found,
/// it validates the request against the signed session and current auth epoch.
/// Missing or empty auth state redirects to first-run; invalid sessions redirect
/// to login.
pub async fn auth_layer(
    jar: CookieJar,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            return Redirect::temporary("/first-run.html").into_response();
        }
        AuthBootstrapState::CorruptUsersFile => {
            return Redirect::temporary("/login.html").into_response();
        }
        AuthBootstrapState::Ready => {}
    }

    let login_result = match session_from_cookie(&jar, &snapshot) {
        Ok(user) => login_result_for_session(user),
        Err(status) => return (status, "Unable to validate session").into_response(),
    };

    match login_result {
        LoginResult::Admin | LoginResult::ReadOnly => {
            record_first_login_timestamp_if_needed();
            let path = req.uri().path().to_string();
            if path.ends_with(".html")
                && !runtime_onboarding_exempt_path(&path)
                && runtime_onboarding_state().required
            {
                return Redirect::temporary("/setup_runtime.html").into_response();
            }
            req.extensions_mut().insert(login_result);
            next.run(req).await
        }
        LoginResult::Denied => Redirect::temporary("/login.html").into_response(),
    }
}

pub async fn login_from_token(token: &str) -> LoginResult {
    let snapshot = auth_snapshot();
    if snapshot.bootstrap_state != AuthBootstrapState::Ready {
        return LoginResult::Denied;
    }

    let key = match session_key() {
        Ok(key) => key,
        Err(e) => {
            warn!("Unable to load session key for websocket auth: {e}");
            return LoginResult::Denied;
        }
    };

    let login_result = match verify_signed_session(&key, token, &snapshot) {
        Ok(user) => login_result_for_session(user),
        Err(e) => {
            warn!("Unable to verify websocket session token: {e}");
            LoginResult::Denied
        }
    };

    if login_result != LoginResult::Denied {
        record_first_login_timestamp_if_needed();
    }

    login_result
}

/// Validates the `User-Token` value from an HTTP Cookie header for websocket upgrades.
pub async fn login_from_cookie_header(cookie_header: Option<&str>) -> LoginResult {
    let Some(token) = session_token_from_cookie_header(cookie_header) else {
        return LoginResult::Denied;
    };
    login_from_token(token).await
}

fn session_token_from_cookie_header(cookie_header: Option<&str>) -> Option<&str> {
    let header = cookie_header?;
    header.split(';').find_map(|entry| {
        let (name, value) = entry.trim().split_once('=')?;
        if name == COOKIE_NAME {
            Some(value)
        } else {
            None
        }
    })
}

/// Invalidate the cached auth snapshot after user-management changes.
pub fn invalidate_auth_cache() {
    let mut lock = AUTH_SNAPSHOT.lock();
    *lock = None;
}

/// Reload the cached auth snapshot after user-management changes.
pub async fn refresh_cached_users() {
    invalidate_auth_cache();
}

#[derive(Serialize, Deserialize)]
pub struct LoginAttempt {
    pub username: String,
    pub password: String,
}

pub async fn try_login(
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    Json(login): Json<LoginAttempt>,
) -> Result<(CookieJar, Json<LoginResponse>), (StatusCode, Json<LoginResponse>)> {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("first_run_required"),
                    message: Some("No users are configured yet.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::CorruptUsersFile => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("The auth file is corrupt and must be repaired.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::Ready => {}
    }

    let remote_ip = remote_addr.ip();
    let rate_limit_username = login_rate_limit_username(&login.username);
    let now = Instant::now();
    if let Some(limit) = LOGIN_RATE_LIMITER
        .lock()
        .check(remote_ip, &rate_limit_username, now)
    {
        warn!(
            remote_ip = %remote_ip,
            username = %rate_limit_username,
            scope = ?limit.scope,
            failures = limit.failures,
            "Rate-limited WebUI login attempt"
        );
        return Err(login_rate_limit_response());
    }

    let mut users = WebUsers::load_or_create().map_err(|e| {
        warn!("Unable to load users during login: {e}");
        (
            StatusCode::CONFLICT,
            Json(LoginResponse {
                ok: false,
                reason: Some("auth_corrupt"),
                message: Some("The auth file is corrupt and must be repaired.".to_string()),
            }),
        )
    })?;
    let authenticated = users
        .authenticate(&login.username, &login.password)
        .map_err(|_| {
            let record =
                LOGIN_RATE_LIMITER
                    .lock()
                    .record_failure(remote_ip, &rate_limit_username, now);
            if record.ip_failures >= LOGIN_REPEATED_FAILURE_LOG_THRESHOLD
                || record.username_failures >= LOGIN_REPEATED_FAILURE_LOG_THRESHOLD
            {
                warn!(
                    remote_ip = %remote_ip,
                    username = %rate_limit_username,
                    ip_failures = record.ip_failures,
                    username_failures = record.username_failures,
                    "Repeated failed WebUI login attempt"
                );
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("invalid_credentials"),
                    message: Some("Invalid username or password.".to_string()),
                }),
            )
        })?;

    LOGIN_RATE_LIMITER
        .lock()
        .clear_username(&rate_limit_username);
    invalidate_auth_cache();
    let key = session_key().map_err(|e| {
        error!("Unable to load session key during login: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;
    let token = build_signed_session(&key, &authenticated).map_err(|status| {
        (
            status,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;

    record_first_login_timestamp_if_needed();
    Ok((
        jar.add(build_session_cookie(token)),
        Json(LoginResponse {
            ok: true,
            reason: None,
            message: None,
        }),
    ))
}

#[derive(Serialize, Deserialize)]
pub struct FirstUser {
    username: String,
    password: String,
}

pub async fn first_user(
    jar: CookieJar,
    Json(new_user): Json<FirstUser>,
) -> Result<(CookieJar, Json<LoginResponse>), (StatusCode, Json<LoginResponse>)> {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::Ready => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("already_configured"),
                    message: Some("Web authentication is already configured.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::CorruptUsersFile => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("The auth file is corrupt and must be repaired.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {}
    }

    let mut users = WebUsers::load_or_create().map_err(|e| {
        warn!("Unable to load users during first-run setup: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("auth_corrupt"),
                message: Some("Unable to initialize auth storage.".to_string()),
            }),
        )
    })?;
    users
        .add_or_update_user(&new_user.username, &new_user.password, UserRole::Admin)
        .map_err(|e| {
            warn!("Unable to create first user: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("Unable to create the first user.".to_string()),
                }),
            )
        })?;

    invalidate_auth_cache();
    let authenticated = AuthenticatedUser {
        username: new_user.username,
        role: UserRole::Admin,
        auth_epoch: users.auth_epoch(),
        password_upgraded: false,
    };
    let key = session_key().map_err(|e| {
        error!("Unable to load session key during first-run setup: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;
    let token = build_signed_session(&key, &authenticated).map_err(|status| {
        (
            status,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;

    record_first_login_timestamp_if_needed();
    Ok((
        jar.add(build_session_cookie(token)),
        Json(LoginResponse {
            ok: true,
            reason: None,
            message: None,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_session_is_denied() {
        assert_eq!(login_result_for_session(None), LoginResult::Denied);
    }

    #[test]
    fn authenticated_read_only_session_keeps_read_only_role() {
        let user = SessionUser {
            username: "support".to_string(),
            role: UserRole::ReadOnly,
        };

        assert_eq!(login_result_for_session(Some(user)), LoginResult::ReadOnly);
    }

    #[test]
    fn login_rate_limiter_blocks_after_failed_attempt_limit() {
        let mut limiter = LoginRateLimiter::new();
        let remote_ip = IpAddr::from([203, 0, 113, 10]);
        let username = "admin";
        let now = Instant::now();

        for attempt in 1..=LOGIN_FAILURE_LIMIT {
            assert_eq!(limiter.check(remote_ip, username, now), None);
            let record = limiter.record_failure(remote_ip, username, now);
            assert_eq!(record.ip_failures, attempt);
            assert_eq!(record.username_failures, attempt);
        }

        assert_eq!(
            limiter.check(remote_ip, username, now),
            Some(LoginRateLimitExceeded {
                scope: LoginRateLimitScope::Ip,
                failures: LOGIN_FAILURE_LIMIT
            })
        );
    }

    #[test]
    fn login_rate_limiter_blocks_username_across_source_ips() {
        let mut limiter = LoginRateLimiter::new();
        let username = "admin";
        let now = Instant::now();

        for host in 1..=LOGIN_FAILURE_LIMIT {
            let remote_ip = IpAddr::from([198, 51, 100, host as u8]);
            assert_eq!(limiter.check(remote_ip, username, now), None);
            limiter.record_failure(remote_ip, username, now);
        }

        let fresh_remote_ip = IpAddr::from([198, 51, 100, 200]);
        assert_eq!(
            limiter.check(fresh_remote_ip, username, now),
            Some(LoginRateLimitExceeded {
                scope: LoginRateLimitScope::Username,
                failures: LOGIN_FAILURE_LIMIT
            })
        );
    }

    #[test]
    fn login_rate_limiter_expires_old_failures_and_clears_on_success() {
        let mut limiter = LoginRateLimiter::new();
        let remote_ip = IpAddr::from([192, 0, 2, 44]);
        let username = "admin";
        let now = Instant::now();

        for _ in 0..LOGIN_FAILURE_LIMIT {
            limiter.record_failure(remote_ip, username, now);
        }
        assert!(limiter.check(remote_ip, username, now).is_some());

        let later = now + LOGIN_FAILURE_WINDOW + Duration::from_secs(1);
        assert_eq!(limiter.check(remote_ip, username, later), None);
        assert!(limiter.by_ip.is_empty());
        assert!(limiter.by_username.is_empty());

        for _ in 0..LOGIN_FAILURE_LIMIT {
            limiter.record_failure(remote_ip, username, later);
        }
        assert!(limiter.check(remote_ip, username, later).is_some());

        limiter.clear_username(username);
        assert_eq!(
            limiter.check(remote_ip, username, later),
            Some(LoginRateLimitExceeded {
                scope: LoginRateLimitScope::Ip,
                failures: LOGIN_FAILURE_LIMIT
            })
        );

        let later_after_window = later + LOGIN_FAILURE_WINDOW + Duration::from_secs(1);
        assert_eq!(limiter.check(remote_ip, username, later_after_window), None);
    }

    #[test]
    fn login_rate_limit_username_normalizes_and_caps_input() {
        assert_eq!(login_rate_limit_username("  Admin  "), "admin");
        assert_eq!(login_rate_limit_username("   "), "<empty>");

        let long_name = "A".repeat(LOGIN_RATE_LIMIT_USERNAME_MAX_CHARS + 10);
        assert_eq!(
            login_rate_limit_username(&long_name).chars().count(),
            LOGIN_RATE_LIMIT_USERNAME_MAX_CHARS
        );
    }

    #[test]
    fn session_cookie_is_http_only_and_secure_when_requested() {
        let cookie = build_session_cookie_with_secure("session-value".to_string(), true);
        let rendered = cookie.to_string();

        assert!(rendered.contains("HttpOnly"));
        assert!(rendered.contains("SameSite=Lax"));
        assert!(rendered.contains("Secure"));
    }

    #[test]
    fn session_cookie_omits_secure_for_direct_http_mode() {
        let cookie = build_session_cookie_with_secure("session-value".to_string(), false);
        let rendered = cookie.to_string();

        assert!(rendered.contains("HttpOnly"));
        assert!(rendered.contains("SameSite=Lax"));
        assert!(!rendered.contains("Secure"));
    }

    #[test]
    fn session_cookie_header_parser_extracts_user_token() {
        assert_eq!(
            session_token_from_cookie_header(Some(
                "Theme=dark; User-Token=v1.payload.sig; other=1"
            )),
            Some("v1.payload.sig")
        );
        assert_eq!(session_token_from_cookie_header(Some("Theme=dark")), None);
        assert_eq!(session_token_from_cookie_header(None), None);
    }
}
