//! Provides authentication for the Node Manager.

use crate::node_manager::locale::{request_ui_locale, set_content_language};
use crate::node_manager::runtime_onboarding::runtime_onboarding_state;
use crate::node_manager::security_headers::{
    apply_node_manager_security_headers, insight_csp_origin,
};
use crate::node_manager::template::apply_page_templates;
use axum::Json;
use axum::extract::ConnectInfo;
use axum::http::header;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
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
const FIRST_RUN_RATE_LIMIT_KEY: &str = "__first_run__";
static FIRST_RUN_RATE_LIMITER: Lazy<Mutex<LoginRateLimiter>> =
    Lazy::new(|| Mutex::new(LoginRateLimiter::new()));

/// Security card 94b00da77 §3.3 (P13): an IN-PROCESS mutex. lqosd already
/// holds a process-lifetime single-instance lock and the state dir is
/// operator-configurable, so a second `<state>/first_run.lock` flock is wrong;
/// the mutex serializes the optimistic verify and the create in this process.
static FIRST_RUN_CREATE_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Normalize a v4-mapped IPv6 TCP peer to a plain v4 address for rate limiting.
fn normalized_peer_ip(peer: &SocketAddr) -> IpAddr {
    match peer.ip() {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        ip => ip,
    }
}

/// DNS-rebinding defense (card §3.2): when `[ssl] external_hostname` is
/// configured, the Host (and any Origin) must match it. With no configured
/// host the check is NOT enforced — the compatibility default; the one-time
/// code remains the gate.
/// The host part of a `Host`/`Origin` authority, without the port. Handles the
/// bracketed IPv6 literal form (`[::1]:9123` -> `::1`); a naive `split(':')`
/// leaves a bare `[` and breaks the strict-mode comparison.
fn host_without_port(authority: &str) -> String {
    let authority = authority.trim().trim_end_matches('.');
    if let Some(rest) = authority.strip_prefix('[') {
        return rest
            .split(']')
            .next()
            .unwrap_or(rest)
            .trim_end_matches('.')
            .to_ascii_lowercase();
    }
    authority
        .split(':')
        .next()
        .unwrap_or(authority)
        .to_ascii_lowercase()
}

fn first_run_host_allowed(headers: &HeaderMap) -> Result<(), &'static str> {
    let Ok(config) = load_config() else {
        return Ok(());
    };
    let Some(expected) = config
        .ssl
        .as_ref()
        .and_then(|ssl| ssl.external_hostname.as_deref())
    else {
        // Advisory by default: log the mode ONCE (never reject). With no
        // external hostname there is no FQDN gate — the code is the control.
        static ADVISORY_LOGGED: std::sync::Once = std::sync::Once::new();
        ADVISORY_LOGGED.call_once(|| {
            warn!(
                "First-run Host/Origin check is advisory (no [ssl] external_hostname configured); the one-time setup code is the gate"
            );
        });
        return Ok(());
    };
    let expected = expected.trim().trim_end_matches('.').to_ascii_lowercase();
    if expected.is_empty() {
        return Ok(());
    }
    // Strict mode accepts the configured external hostname OR the operator's
    // configured listen host (the bind): a browser that reaches the WebUI by
    // the bind IP must not be rejected.
    let listen_host = config
        .webserver_listen
        .as_deref()
        .map(host_without_port)
        .filter(|host| !host.is_empty());
    let allowed = |host_name: &str| {
        host_name == expected || listen_host.as_deref() == Some(host_name)
    };
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let host_name = host_without_port(host);
    if !allowed(&host_name) {
        return Err("host_mismatch");
    }
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let authority = origin
            .split("://")
            .nth(1)
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        let origin_host = host_without_port(authority);
        if !origin_host.is_empty() && !allowed(&origin_host) {
            return Err("origin_mismatch");
        }
    }
    Ok(())
}

/// Security card §3.1: generate the one-time code at startup/reset ONLY while
/// the instance is unconfigured. Fail-closed and loud.
pub fn ensure_first_run_setup_code() {
    let state = auth_snapshot().bootstrap_state;
    if matches!(state, AuthBootstrapState::Ready) {
        // A configured instance never keeps a one-time code around: remove any
        // stale artifact (e.g. the admin was created via `lqusers`). A failed
        // purge is loud, not ignored.
        if let Err(error) = crate::node_manager::setup_code::consume() {
            // A stale code surviving the purge must never become usable if the
            // instance is later reset in-process: latch the request path closed.
            crate::node_manager::setup_code::mark_unavailable();
            error!(
                "First-run setup code purge on Ready FAILED ({error}); the first-run flow will refuse (fail closed)"
            );
        }
        return;
    }
    if !matches!(
        state,
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured
    ) {
        return;
    }
    match crate::node_manager::setup_code::generate_unconfigured() {
        Ok(_path) => {
            // Startup established the code: the request path may verify it.
            crate::node_manager::setup_code::mark_available();
            crate::node_manager::setup_code::log_location();
        }
        Err(error) => {
            // Fail closed: a stale file surviving a failed remove/create must
            // never be accepted. The handler answers 503 while this is latched.
            crate::node_manager::setup_code::mark_unavailable();
            error!(
                "First-run setup code generation FAILED ({error}); the first-run flow will refuse (fail closed)"
            );
        }
    }
}

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

fn serve_standalone_page(
    page: &str,
    jar: &CookieJar,
    headers: &HeaderMap,
) -> Result<Response, StatusCode> {
    let config = load_config().map_err(|e| {
        error!("Unable to load config for standalone page {page}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join(page);
    let body = std::fs::read_to_string(&path).map_err(|e| {
        error!("Unable to read standalone page {:?}: {e}", path);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let locale = request_ui_locale(config.ui_language.as_deref(), jar, headers);
    let mut response = Html(apply_page_templates(&body, &locale)).into_response();
    let response_headers = response.headers_mut();
    // These pages now vary by cookie and by language, and they are public: a
    // shared cache must not replay one browser's language choice to the next.
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(
        header::VARY,
        HeaderValue::from_static("cookie, accept-language"),
    );
    set_content_language(response_headers, &locale)?;
    let extra = insight_csp_origin(config.long_term_stats.lts_url.as_deref());
    apply_node_manager_security_headers(response_headers, extra.as_deref());
    Ok(response)
}

pub async fn login_page(jar: CookieJar, headers: HeaderMap) -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            Redirect::temporary("/first-run.html").into_response()
        }
        AuthBootstrapState::Ready => match session_from_cookie(&jar, &snapshot) {
            Ok(Some(_)) => Redirect::temporary(post_login_destination()).into_response(),
            Ok(None) => serve_standalone_page("login.html", &jar, &headers)
                .unwrap_or_else(|status| (status, "Unable to serve login page").into_response()),
            Err(status) => (status, "Unable to validate login session").into_response(),
        },
        AuthBootstrapState::CorruptUsersFile => serve_standalone_page("login.html", &jar, &headers)
            .unwrap_or_else(|status| (status, "Unable to serve login page").into_response()),
    }
}

pub async fn first_run_page(jar: CookieJar, headers: HeaderMap) -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            serve_standalone_page("first-run.html", &jar, &headers)
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

/// E3: the session's role for UI affordance gating ("Admin" sees the
/// admin-action surfaces; anything else — including anonymous — is
/// read-only). The BACKEND remains the authority: write endpoints
/// re-check the session and reject non-Admin regardless of what the
/// UI rendered.
pub async fn get_role(jar: &CookieJar) -> &'static str {
    let snapshot = auth_snapshot();
    match session_from_cookie(jar, &snapshot) {
        Ok(Some(user)) => match user.role {
            UserRole::Admin => "Admin",
            UserRole::ReadOnly => "ReadOnly",
        },
        Ok(None) | Err(_) => "Anonymous",
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

/// Reset every process-global first-run/auth cache between tests. The handler's
/// state is process-wide (snapshot, session key, failure limiter, the
/// unavailable latch), so handler tests must run serialized and start clean.
#[cfg(test)]
pub(crate) fn reset_first_run_state_for_test() {
    invalidate_auth_cache();
    *SESSION_KEY.lock() = None;
    *FIRST_RUN_RATE_LIMITER.lock() = LoginRateLimiter::new();
    crate::node_manager::setup_code::reset_latch_for_test();
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
    /// Security card 1ad21622a: the one-time setup code (POST body). A
    /// dedicated `X-Setup-Code` header is also accepted; the query is NEVER
    /// read. Missing/empty => refused (never an implicit bypass).
    #[serde(default)]
    setup_code: Option<String>,
}

pub async fn first_user(
    jar: CookieJar,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(new_user): Json<FirstUser>,
) -> Result<(CookieJar, Json<LoginResponse>), (StatusCode, Json<LoginResponse>)> {
    let remote_ip = normalized_peer_ip(&peer);

    // Advisory pre-check: Ready/Corrupt refuse WITHOUT consulting the code.
    match auth_snapshot().bootstrap_state {
        AuthBootstrapState::Ready => {
            warn!(ip = %remote_ip, outcome = "refused", reason = "already_configured", "first-run request");
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
            warn!(ip = %remote_ip, outcome = "refused", reason = "auth_corrupt", "first-run request");
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

    // There is NO app-level source gate: the BIND is the operator's choice.
    // beta20.8 (driver ruling 2026-11-14) restores the pre-Nov-11 default
    // (`webserver_listen` absent/blank => 0.0.0.0:9123); the direct listener
    // relies on the one-time CODE + login auth, exactly as a deliberate remote
    // bind always has. An explicit `webserver_listen` overrides it. XFF is
    // never a decision input.

    // Fail closed: startup could not establish a usable code (a stale file may
    // still exist). Refuse rather than accept an unknown code.
    if crate::node_manager::setup_code::is_unavailable() {
        warn!(
            ip = %remote_ip,
            outcome = "refused",
            reason = "first_run_unavailable",
            "first-run request"
        );
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(LoginResponse {
                ok: false,
                reason: Some("first_run_unavailable"),
                message: Some(
                    "The one-time setup code could not be established on the server.".to_string(),
                ),
            }),
        ));
    }

    // DNS-rebinding defense (enforced only when an expected host is configured).
    if let Err(reason) = first_run_host_allowed(&headers) {
        warn!(ip = %remote_ip, outcome = "refused", reason = %reason, "first-run request");
        return Err((
            StatusCode::FORBIDDEN,
            Json(LoginResponse {
                ok: false,
                reason: Some("host_not_allowed"),
                message: Some("This first-run request did not match the configured host.".to_string()),
            }),
        ));
    }

    // P12 (card 94b00da77): the CODE is verified FIRST, so a CORRECT code is
    // never deniable. The rate limiter is consulted only after a FAILED
    // verification (below); behind Caddy every peer is loopback, so a naive
    // pre-check would be an operator-lockout DoS.

    // The code comes from the POST body or a dedicated header; NEVER the query.
    let candidate = new_user
        .setup_code
        .as_deref()
        .or_else(|| headers.get("x-setup-code").and_then(|v| v.to_str().ok()))
        .unwrap_or("")
        .trim()
        .to_string();
    if candidate.is_empty() || !crate::node_manager::setup_code::verify(&candidate) {
        // A FAILED attempt is the only thing the limiter counts; a correct
        // code has already returned through the success path and is never
        // deniable (P12).
        let now = Instant::now();
        let mut limiter = FIRST_RUN_RATE_LIMITER.lock();
        limiter.record_failure(remote_ip, FIRST_RUN_RATE_LIMIT_KEY, now);
        if let Some(exceeded) = limiter.check(remote_ip, FIRST_RUN_RATE_LIMIT_KEY, now) {
            warn!(
                ip = %remote_ip,
                outcome = "rate_limited",
                scope = ?exceeded.scope,
                failures = exceeded.failures,
                "first-run request"
            );
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("rate_limited"),
                    message: Some("Too many first-run attempts. Try again shortly.".to_string()),
                }),
            ));
        }
        drop(limiter);
        warn!(ip = %remote_ip, outcome = "refused", reason = "invalid_setup_code", "first-run request");
        return Err((
            StatusCode::FORBIDDEN,
            Json(LoginResponse {
                ok: false,
                reason: Some("invalid_setup_code"),
                message: Some(
                    "A valid one-time setup code is required. Find it on the server (see startup log)."
                        .to_string(),
                ),
            }),
        ));
    }

    // Serialize the create: the IN-PROCESS mutex (P13) + a FRESH under-mutex
    // re-check of the authoritative on-disk state. No lock file is created.
    let _create_guard = FIRST_RUN_CREATE_MUTEX.lock();
    invalidate_auth_cache();
    match auth_snapshot().bootstrap_state {
        AuthBootstrapState::Ready => {
            warn!(ip = %remote_ip, outcome = "refused", reason = "already_configured", "first-run request");
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
            warn!(ip = %remote_ip, outcome = "refused", reason = "auth_corrupt", "first-run request");
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
    // Re-verify under the mutex: a concurrent winner may have consumed/rotated
    // the code between the optimistic verify and mutex acquisition.
    if !crate::node_manager::setup_code::verify(&candidate) {
        warn!(ip = %remote_ip, outcome = "refused", reason = "invalid_setup_code", "first-run request");
        return Err((
            StatusCode::FORBIDDEN,
            Json(LoginResponse {
                ok: false,
                reason: Some("invalid_setup_code"),
                message: Some("The one-time setup code is no longer valid.".to_string()),
            }),
        ));
    }

    // P7 (card 94b00da77): DURABLY consume the code BEFORE create. A crash
    // after the consume leaves NO live code and NO admin: the next request
    // refuses (no code), and a restart of the still-unconfigured instance
    // generates a fresh code. Consuming after the create could re-arm it on a
    // crash — exactly the P7 failure.
    if let Err(e) = crate::node_manager::setup_code::consume_durable() {
        error!(ip = %remote_ip, "Unable to durably consume first-run setup code: {e}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("first_run_code"),
                message: Some("Unable to consume the one-time setup code.".to_string()),
            }),
        ));
    }

    let mut users = WebUsers::load_or_create().map_err(|e| {
        warn!(ip = %remote_ip, "Unable to load users during first-run setup: {e}");
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
            warn!(ip = %remote_ip, "Unable to create first user: {e}");
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
    warn!(
        ip = %remote_ip,
        outcome = "created",
        user = %new_user.username,
        "first-run setup completed"
    );
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
    fn first_run_peer_normalizes_v4_mapped_ipv6() {
        let mapped = SocketAddr::new(
            IpAddr::V6(std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001)),
            443,
        );
        assert_eq!(normalized_peer_ip(&mapped), IpAddr::from([127, 0, 0, 1]));

        let native = SocketAddr::new(
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            443,
        );
        assert_eq!(normalized_peer_ip(&native), IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));

        let v4 = SocketAddr::new(IpAddr::from([192, 0, 2, 9]), 443);
        assert_eq!(normalized_peer_ip(&v4), IpAddr::from([192, 0, 2, 9]));
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

/// Security card 94b00da77 §3.5: the `/firstLogin` handler test harness and the
/// 12 required handler tests. Everything here is test-only: a temp install dir,
/// an injectable code path, `ConnectInfo` via `MockConnectInfo`, and the
/// process-global auth caches reset between tests (the tests are serialized by
/// the shared runtime-config lock because env + caches are process-wide).
#[cfg(test)]
mod first_run_handler_tests {
    use super::*;
    use crate::node_manager::setup_code;
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::Request;
    use axum::http::header;
    use axum::routing::post;
    use axum::Router;
    use std::ffi::OsString;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    /// A 32-byte hex code (the fixed production length).
    const CODE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const WRONG: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn loopback_peer() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 4242))
    }

    fn remote_peer() -> SocketAddr {
        SocketAddr::from(([203, 0, 113, 9], 4242))
    }

    fn current_uid() -> u32 {
        unsafe { libc::getuid() }
    }

    fn login(user: &str, code: Option<&str>) -> serde_json::Value {
        match code {
            Some(code) => serde_json::json!({
                "username": user,
                "password": "correct horse battery staple",
                "setup_code": code,
            }),
            None => serde_json::json!({
                "username": user,
                "password": "correct horse battery staple",
            }),
        }
    }

    /// Temp install dir + config seam + code-path seam + ConnectInfo. Holds the
    /// shared runtime-config lock, so the process-global env/caches are stable
    /// for the life of the fixture.
    struct FirstRunEnv {
        _guard: std::sync::MutexGuard<'static, ()>,
        dir: PathBuf,
        runtime_dir: PathBuf,
        code_dir: PathBuf,
        old_config: Option<OsString>,
        old_directory: Option<OsString>,
    }

    impl FirstRunEnv {
        fn new(label: &str) -> Self {
            Self::with_runtime(label, None)
        }

        fn with_runtime(label: &str, runtime: Option<&Path>) -> Self {
            let guard = crate::test_support::runtime_config_test_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let dir = std::env::temp_dir().join(format!(
                "lqosd-first-run-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            fs::create_dir_all(dir.join("state")).expect("create test install dir");
            let runtime_dir = runtime.map(Path::to_path_buf).unwrap_or_else(|| dir.clone());
            let config_path = dir.join("lqos.conf");
            let raw = include_str!("../../../lqos_config/src/etc/v15/example.toml")
                .replacen(
                    "lqos_directory = \"/opt/libreqos/src\"",
                    &format!("lqos_directory = {:?}", dir.to_string_lossy()),
                    1,
                )
                .replacen(
                    "state_directory = \"/opt/libreqos/state\"",
                    &format!(
                        "state_directory = {:?}",
                        dir.join("state").to_string_lossy()
                    ),
                    1,
                );
            fs::write(&config_path, raw).expect("write test lqos.conf");

            let old_config = std::env::var_os("LQOS_CONFIG");
            let old_directory = std::env::var_os("LQOS_DIRECTORY");
            unsafe {
                std::env::set_var("LQOS_CONFIG", &config_path);
                std::env::set_var("LQOS_DIRECTORY", &runtime_dir);
            }
            lqos_config::clear_cached_config();

            let code_dir = dir.join("first_run");
            setup_code::set_test_code_dir(Some(code_dir.clone()));
            setup_code::set_test_required_uid(current_uid());
            reset_first_run_state_for_test();

            Self {
                _guard: guard,
                dir,
                runtime_dir,
                code_dir,
                old_config,
                old_directory,
            }
        }

        fn write_code(&self, code: &str) {
            setup_code::write_code_for_test(&self.code_dir, code).expect("write test code");
        }

        fn code_file(&self) -> PathBuf {
            self.code_dir.join(setup_code::CODE_FILE)
        }

        fn code_exists(&self) -> bool {
            self.code_file().exists()
        }

        fn read_code(&self) -> String {
            fs::read_to_string(self.code_file())
                .expect("read test code")
                .trim()
                .to_string()
        }

        fn users_path(&self) -> PathBuf {
            self.runtime_dir.join("lqusers.toml")
        }

        fn app(&self, peer: SocketAddr) -> Router {
            Router::new()
                .route("/firstLogin", post(first_user))
                .layer(MockConnectInfo(peer))
        }

        async fn post(
            &self,
            peer: SocketAddr,
            body: serde_json::Value,
            extra_headers: &[(&str, &str)],
        ) -> (StatusCode, axum::http::HeaderMap, String) {
            let mut builder = Request::builder()
                .method("POST")
                .uri("/firstLogin")
                .header(header::CONTENT_TYPE, "application/json");
            for (name, value) in extra_headers {
                builder = builder.header(*name, *value);
            }
            let request = builder
                .body(Body::from(body.to_string()))
                .expect("build request");
            let response = self.app(peer).oneshot(request).await.expect("handler answers");
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("read body");
            (status, headers, String::from_utf8_lossy(&bytes).into_owned())
        }
    }

    impl Drop for FirstRunEnv {
        fn drop(&mut self) {
            match &self.old_config {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            match &self.old_directory {
                Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
                None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
            }
            lqos_config::clear_cached_config();
            setup_code::set_test_code_dir(None);
            setup_code::set_test_required_uid(u32::MAX);
            reset_first_run_state_for_test();
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[derive(Clone, Default)]
    struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl CapturedLog {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl std::io::Write for CapturedLog {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
        type Writer = CapturedLog;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// (1) Unconfigured + no/wrong code => refused, no user.
    #[tokio::test]
    async fn missing_and_wrong_codes_are_refused_without_creating_a_user() {
        let env = FirstRunEnv::new("refuse");

        let (status, _, body) = env.post(loopback_peer(), login("admin", None), &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.contains("invalid_setup_code"));
        assert!(!env.users_path().exists());

        env.write_code(CODE);
        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(WRONG)), &[])
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(!body.contains(CODE));
        assert!(!body.contains(WRONG));
        assert!(!env.users_path().exists());
        assert!(env.code_exists(), "a failed verify must not consume the code");
    }

    /// (2) Correct code => 200 + Admin + consumed code.
    #[tokio::test]
    async fn a_correct_code_creates_one_admin_and_consumes_the_code() {
        let env = FirstRunEnv::new("create");
        env.write_code(CODE);

        let (status, headers, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!env.code_exists(), "the code must be consumed");
        assert!(!body.contains(CODE));

        let users = WebUsers::load_or_create_in(&env.runtime_dir).expect("users file");
        let list = users.get_users();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].username, "admin");
        assert_eq!(list[0].role, UserRole::Admin);

        let cookie = headers
            .get(header::SET_COOKIE)
            .expect("a session cookie")
            .to_str()
            .unwrap();
        assert!(cookie.contains("User-Token="));
        assert!(!cookie.contains(CODE));
    }

    /// (3) N parallel correct-code requests => EXACTLY ONE Admin.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_correct_code_requests_create_exactly_one_admin() {
        let env = FirstRunEnv::new("parallel");
        env.write_code(CODE);
        let app = env.app(loopback_peer());

        let mut set = tokio::task::JoinSet::new();
        for index in 0..8 {
            let app = app.clone();
            set.spawn(async move {
                let body = serde_json::json!({
                    "username": format!("admin{index}"),
                    "password": "correct horse battery staple",
                    "setup_code": CODE,
                })
                .to_string();
                let request = Request::builder()
                    .method("POST")
                    .uri("/firstLogin")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap();
                app.oneshot(request).await.expect("handler answers").status()
            });
        }
        let mut statuses = Vec::new();
        while let Some(result) = set.join_next().await {
            statuses.push(result.expect("join"));
        }

        assert_eq!(
            statuses.iter().filter(|s| **s == StatusCode::OK).count(),
            1,
            "exactly one create: {statuses:?}"
        );
        let users = WebUsers::load_or_create_in(&env.runtime_dir).expect("users file");
        assert_eq!(users.get_users().len(), 1);
        assert!(
            !env.dir.join("first_run.lock").exists(),
            "P13: the in-process mutex must not create a lock file"
        );
    }

    /// (4) A mid-create failure leaves no partial users file (P7: the code is
    /// already durably consumed).
    #[tokio::test]
    async fn a_failed_create_consumes_the_code_and_leaves_no_users_file() {
        // `/sys/kernel` exists but cannot host a new regular file (even as
        // root), so the save fails at the write/rename step.
        let env = FirstRunEnv::with_runtime("create-fail", Some(Path::new("/sys/kernel")));
        env.write_code(CODE);

        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert!(!env.code_exists(), "P7: consume before create");
        assert!(!env.users_path().exists(), "no partial users file");
    }

    /// (5) Driver ruling 2026-11-12: the BIND is the network control; there is
    /// no app-level source gate. A deliberate remote bind therefore relies on
    /// the one-time code alone: a remote peer with a wrong code is refused, and
    /// with the correct code it succeeds. XFF is never a decision input.
    #[tokio::test]
    async fn a_deliberate_remote_bind_relies_on_the_code_alone() {
        let env = FirstRunEnv::new("remote-bind");
        env.write_code(CODE);

        let (status, _, body) = env
            .post(
                remote_peer(),
                login("admin", Some(WRONG)),
                &[("x-forwarded-for", "127.0.0.1")],
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.contains("invalid_setup_code"));
        assert!(env.code_exists(), "a failed verify must not consume the code");

        let (status, _, body) = env
            .post(remote_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a deliberate remote bind must rely on the code: {body}"
        );
    }

    /// (5b) Strict-mode Host parsing must handle bracketed IPv6 literals; a
    /// naive `split(':')` leaves `[` and breaks the comparison.
    #[test]
    fn host_header_parsing_handles_ipv6_literals_and_ports() {
        assert_eq!(host_without_port("[::1]:9123"), "::1");
        assert_eq!(host_without_port("[2001:db8::1]"), "2001:db8::1");
        assert_eq!(host_without_port("Example.COM:9123"), "example.com");
        assert_eq!(host_without_port("example.com."), "example.com");
    }

    /// (6) The limiter denies FAILED attempts only; a correct code after a
    /// failure storm still succeeds (P12).
    #[tokio::test]
    async fn the_limiter_counts_failed_attempts_only_and_a_correct_code_is_never_denied() {
        let env = FirstRunEnv::new("limiter");
        env.write_code(CODE);

        for _ in 0..LOGIN_FAILURE_LIMIT {
            let (status, _, _) = env
                .post(loopback_peer(), login("admin", Some(WRONG)), &[])
                .await;
            assert!(
                matches!(
                    status,
                    StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
                ),
                "{status}"
            );
        }
        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(WRONG)), &[])
            .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a correct code must never be deniable: {body}"
        );
    }

    /// (7) Reset (`rm lqusers.toml` + restart) rotates the code; the old code is
    /// refused and there is no in-process re-arm.
    #[tokio::test]
    async fn a_reset_rotates_the_code_and_never_rearms_it_in_process() {
        let env = FirstRunEnv::new("reset");
        env.write_code(CODE);
        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // No live re-arm of the consumed code.
        let (status, _, body) = env
            .post(loopback_peer(), login("second", Some(CODE)), &[])
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("already_configured"));

        fs::remove_file(env.users_path()).expect("remove users file");
        invalidate_auth_cache();
        ensure_first_run_setup_code();
        let rotated = env.read_code();
        assert_ne!(rotated, CODE);
        assert_eq!(rotated.len(), 64);

        let (status, _, body) = env
            .post(loopback_peer(), login("second", Some(CODE)), &[])
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "the old code must be refused: {body}"
        );
        let (status, _, body) = env
            .post(loopback_peer(), login("second", Some(rotated.as_str())), &[])
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// (8) Ready never consults or creates a code; it purges a stale one.
    #[tokio::test]
    async fn a_ready_instance_never_consults_or_creates_a_code() {
        let env = FirstRunEnv::new("ready");
        let mut users = WebUsers::load_or_create_in(&env.runtime_dir).expect("users");
        users
            .add_or_update_user("admin", "pw", UserRole::Admin)
            .expect("seed admin");
        invalidate_auth_cache();
        assert!(!env.code_exists());

        let (status, _, body) = env.post(loopback_peer(), login("x", Some(CODE)), &[]).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("already_configured"));
        assert!(!env.code_exists(), "Ready must not create a code");

        env.write_code(CODE);
        ensure_first_run_setup_code();
        assert!(!env.code_exists(), "Ready must purge a stale code");
    }

    /// (8, corrupt) A corrupt users file never consults or creates a code.
    #[tokio::test]
    async fn a_corrupt_instance_never_consults_or_creates_a_code() {
        let env = FirstRunEnv::new("corrupt");
        fs::write(env.users_path(), b"this is not toml = [").expect("write corrupt file");
        invalidate_auth_cache();
        assert!(!env.code_exists());

        let (status, _, body) = env.post(loopback_peer(), login("x", Some(CODE)), &[]).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("auth_corrupt"));
        assert!(!env.code_exists(), "Corrupt must not create a code");
    }

    /// (9) The code never appears in the cookie, the response, the log, or the
    /// support bundle.
    #[tokio::test]
    async fn the_code_never_appears_in_the_cookie_response_or_log() {
        let env = FirstRunEnv::new("leak");
        env.write_code(CODE);

        let (_, _, body) = env
            .post(loopback_peer(), login("admin", Some(WRONG)), &[])
            .await;
        assert!(!body.contains(CODE));
        assert!(!body.contains(WRONG));

        let captured = CapturedLog::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let (status, headers, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        setup_code::log_location();
        drop(guard);
        assert_eq!(status, StatusCode::OK, "{body}");

        let cookie = headers
            .get(header::SET_COOKIE)
            .expect("a session cookie")
            .to_str()
            .unwrap();
        assert!(cookie.contains("User-Token="));
        assert!(!cookie.contains(CODE));
        assert!(!body.contains(CODE));

        let log = String::from_utf8(captured.bytes()).unwrap();
        assert!(
            log.contains(setup_code::CODE_FILE),
            "the boot line must name the path: {log}"
        );
        assert!(!log.contains(CODE), "the code must never reach the log: {log}");

        // The support bundle collects journald/bridge state, never /run.
        let bundle = include_str!("../../../../../../packaging/libexec/libreqos-support-bundle");
        assert!(!bundle.contains("/run/lqos"));
        assert!(!bundle.contains("first_run"));
    }

    /// (10) Fail closed when the code cannot be established at startup.
    #[tokio::test]
    async fn an_unestablished_code_fails_closed() {
        let env = FirstRunEnv::new("unavailable");
        let blocker = env.dir.join("blocker");
        fs::write(&blocker, b"x").expect("create blocker file");
        setup_code::set_test_code_dir(Some(blocker.join("first_run")));

        ensure_first_run_setup_code();
        assert!(
            setup_code::is_unavailable(),
            "a failed startup generation must latch the request path closed"
        );

        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(body.contains("first_run_unavailable"));
    }

    /// (11) The UI field contract: a required setup-code input posts
    /// `setup_code` in the JSON body, and the page names the real `/run` path.
    #[test]
    fn the_first_run_ui_requires_the_setup_code_field_and_posts_it_in_the_body() {
        let html = include_str!("static2/first-run.html");
        assert!(html.contains("id=\"setupCode\""));
        assert!(html.contains(setup_code::CODE_DIR));
        assert!(!html.contains("/run/libreqos-first-run"));

        let js = include_str!("js_build/src/first-run.js");
        assert!(js.contains("setup_code: setupCode"));
        assert!(js.contains("url: \"/firstLogin\""));
    }

    /// (14) An empty `lqusers.toml` is `NoUsersConfigured`: first-run is
    /// allowed with the code and takes the same consume-before-create path.
    #[tokio::test]
    async fn an_empty_users_file_is_unconfigured_and_creates_an_admin() {
        let env = FirstRunEnv::new("empty-file");
        fs::write(
            env.users_path(),
            b"version = 2\nauth_epoch = 1\nusers = []\n",
        )
        .expect("write empty users file");
        invalidate_auth_cache();
        env.write_code(CODE);

        let (status, _, body) = env
            .post(loopback_peer(), login("admin", Some(CODE)), &[])
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!env.code_exists(), "the code must be consumed");
        let users = WebUsers::load_or_create_in(&env.runtime_dir).expect("users file");
        assert_eq!(users.get_users().len(), 1);
    }

    /// (5c) `ConnectInfo` accepts an IPv6 loopback peer and the v4-mapped form;
    /// the peer is normalized for rate limiting/logging (no source gate).
    #[tokio::test]
    async fn ipv6_loopback_and_v4_mapped_peers_are_accepted() {
        {
            let env = FirstRunEnv::new("ipv6-loopback");
            env.write_code(CODE);
            let peer = SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, 4242));
            let (status, _, body) = env.post(peer, login("admin", Some(CODE)), &[]).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        {
            let env = FirstRunEnv::new("v4-mapped");
            env.write_code(CODE);
            let peer = SocketAddr::new(
                IpAddr::V6(std::net::Ipv6Addr::new(
                    0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001,
                )),
                4242,
            );
            let (status, _, body) = env.post(peer, login("admin", Some(CODE)), &[]).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    }
}
