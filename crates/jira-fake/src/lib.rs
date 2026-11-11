//! Faux serveur Jira Cloud en mémoire, pour les tests d'intégration de `jira-rust`.
//!
//! Écoute uniquement sur 127.0.0.1. Reproduit le format HTTP de Jira Cloud (REST v3 + Agile 1.0)
//! d'après `docs/api` et les fixtures de `crates/jira-client/tests/fixtures`.

mod jql;
mod routes;
mod state;
mod time;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Request, State as AxState};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;

pub use jql::SUPPORTED as SUPPORTED_JQL;
pub use state::State;

/// Comportements simulés (latence, 429, écritures interdites).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Délai ajouté à chaque requête.
    pub latency: Duration,
    /// Fraction (0.0 à 1.0) des requêtes renvoyant 429.
    pub fail_rate: f64,
    /// Valeur de l'en-tête `Retry-After` (secondes) des 429.
    pub retry_after_secs: u64,
    /// 403 sur toutes les écritures.
    pub forbid_writes: bool,
    /// Pas de log des requêtes sur stderr.
    pub quiet: bool,
}

#[derive(Clone)]
pub(crate) struct App {
    pub state: Arc<Mutex<State>>,
    pub opts: Arc<Options>,
}

impl App {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Erreur au format Jira : `{ "errorMessages": [], "errors": {} }`.
#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    messages: Vec<String>,
    errors: Map<String, Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        Self {
            status,
            messages: vec![msg.into()],
            errors: Map::new(),
        }
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
    /// 400 avec erreur par champ.
    pub fn field(field: &str, msg: impl Into<String>) -> Self {
        let mut errors = Map::new();
        errors.insert(field.into(), json!(msg.into()));
        Self {
            status: StatusCode::BAD_REQUEST,
            messages: vec![],
            errors,
        }
    }
    pub fn fields(errors: Map<String, Value>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            messages: vec![],
            errors,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "errorMessages": self.messages, "errors": self.errors });
        (self.status, axum::Json(body)).into_response()
    }
}

pub(crate) type ApiResult = Result<Response, ApiError>;

fn is_write(method: &Method, path: &str) -> bool {
    match *method {
        Method::GET | Method::HEAD | Method::OPTIONS => false,
        // Recherche JQL : POST en lecture seule.
        Method::POST => path != "/rest/api/3/search/jql",
        _ => true,
    }
}

/// Log, latence, 429 aléatoires, authentification, `--forbid-writes`.
async fn guard(AxState(app): AxState<App>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let target = req
        .uri()
        .path_and_query()
        .map_or_else(|| path.clone(), |p| p.as_str().to_owned());

    if !app.opts.latency.is_zero() {
        tokio::time::sleep(app.opts.latency).await;
    }
    let authorized = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .is_some_and(|cred| !cred.trim().is_empty());

    let resp = if !authorized {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "You are not authenticated. Authentication required to perform this operation.",
        )
        .into_response()
    } else if app.opts.fail_rate > 0.0 && fastrand::f64() < app.opts.fail_rate {
        let mut r = ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "jira-fake : limite de débit simulée (--fail-rate).",
        )
        .into_response();
        let h = r.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&app.opts.retry_after_secs.to_string()) {
            h.insert(header::RETRY_AFTER, v);
        }
        h.insert(
            "RateLimit-Reason",
            HeaderValue::from_static("jira-burst-based"),
        );
        h.insert("X-RateLimit-Remaining", HeaderValue::from_static("0"));
        r
    } else if app.opts.forbid_writes && is_write(&method, &path) {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "jira-fake --forbid-writes : vous n'avez pas la permission d'effectuer cette opération.",
        )
        .into_response()
    } else {
        next.run(req).await
    };

    if !app.opts.quiet {
        eprintln!(
            "jira-fake: {method} {target} -> {} ({} ms)",
            resp.status().as_u16(),
            started.elapsed().as_millis()
        );
    }
    resp
}

async fn fallback() -> ApiError {
    ApiError::not_found("jira-fake : endpoint non simulé.")
}

/// Routeur complet. `base_url` sert à construire les liens `self` (ex. `http://127.0.0.1:8089`).
pub fn router(opts: Options, base_url: &str) -> Router {
    let app = App {
        state: Arc::new(Mutex::new(State::seed(base_url.trim_end_matches('/')))),
        opts: Arc::new(opts),
    };
    routes::routes()
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

/// Lie un port sur 127.0.0.1 (jamais une autre interface). Port 0 = éphémère.
pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
}

/// Sert le faux Jira sur un listener déjà lié (doit être en loopback).
pub async fn serve(
    listener: TcpListener,
    opts: Options,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let addr = listener.local_addr()?;
    if !addr.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("jira-fake n'écoute qu'en loopback (adresse refusée : {addr})"),
        ));
    }
    let app = router(opts, &format!("http://{addr}"));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}
