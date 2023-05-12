mod auth;
mod error;
mod lqos;
mod site;
mod utils;
mod tracker;

use axum::{
    error_handling::HandleErrorLayer,
	extract::{Host, State},
    handler::HandlerWithoutStateExt,
	http::{StatusCode, Uri},
    middleware,
    response::{IntoResponse, Redirect, Response},
    routing::{get, get_service},
    BoxError,
    Router,
};
use axum_server::tls_rustls::RustlsConfig;

use std::{net::SocketAddr, path::PathBuf, sync::{Arc, Mutex}};

use tokio::{
    sync::{broadcast, mpsc::{self, UnboundedSender}},
    time::{Duration, Instant}
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
	cors::{Any, CorsLayer},
	services::ServeDir,
	trace::TraceLayer,
};
use async_channel::{bounded, Receiver, Sender};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::session_layer;
use auth::auth_layer;
use tracker::Tracker;
use crate::site::websocket::{event::WsEvent, message::WsMessage};

// Use JemAllocator only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use jemallocator::Jemalloc;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Clone)]
pub struct AppState {
    pending_reload: bool,
    async_rx: Receiver<WsEvent>,
    tracker: Tracker,
}

#[derive(Clone, Copy)]
struct Ports {
    http: u16,
    https: u16,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lqos_node_manager=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let ports = Ports {
        http: 8080,
        https: 8443,
    };

    tokio::spawn(redirect_http_to_https(ports));

	let cors = CorsLayer::permissive();
    //let redis_client = redis::Client::open("redis://127.0.0.1").unwrap();

    let (async_tx, async_rx) = async_channel::bounded(8);

    // Creates and starts a new tracker with polling interval of 1000ms
    let tracker = Tracker::new(async_tx);
    tracker.start(1000);

	let app_state = AppState {
        pending_reload: false,
        async_rx: async_rx,
        tracker: tracker,
    };

    let app = Router::new()
		.merge(site::main::routes())
		.merge(site::websocket::routes())
		.nest("/tree", site::tree::routes())
		.nest("/circuit", site::circuit::routes())
		.nest("/config", site::config::routes())
		.nest("/auth", site::auth::routes())
        .nest_service("/assets", get_service(ServeDir::new("assets")))
		.layer(ServiceBuilder::new()
            .layer(cors)
            .layer(CompressionLayer::new())
            .layer(session_layer())
            .layer(auth_layer())
            .map_response(|r: Response|{
                if r.status()==StatusCode::UNAUTHORIZED {
                    error::AppError::Unauthorized.into_response()
                } else if r.status()==StatusCode::NOT_FOUND {
                    error::AppError::NotFound.into_response()
                } else if r.status()==StatusCode::INTERNAL_SERVER_ERROR {
                    error::AppError::InternalServerError.into_response()
                } else {
                    r
                }}))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let config = RustlsConfig::from_pem_file("certs/cert.pem", "certs/key.pem")
        .await
        .unwrap();

    let addr = SocketAddr::from(("[::]", ports.https));
    tracing::debug!("listening on {}", addr);
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn redirect_http_to_https(ports: Ports) {
    fn make_https(host: String, uri: Uri, ports: Ports) -> Result<Uri, BoxError> {
        let mut parts = uri.into_parts();
        parts.scheme = Some(axum::http::uri::Scheme::HTTPS);
        if parts.path_and_query.is_none() {
            parts.path_and_query = Some("/".parse().unwrap());
        }
        let https_host = host.replace(&ports.http.to_string(), &ports.https.to_string());
        parts.authority = Some(https_host.parse()?);
        Ok(Uri::from_parts(parts)?)
    }

    let redirect = move |Host(host): Host, uri: Uri| async move {
        match make_https(host, uri, ports) {
            Ok(uri) => Ok(Redirect::permanent(&uri.to_string())),
            Err(error) => {
                tracing::warn!(%error, "failed to convert URI to HTTPS");
                Err(StatusCode::BAD_REQUEST)
            }
        }
    };

    let addr = SocketAddr::from(("[::]", ports.http));
    tracing::debug!("http redirect listening on {}", addr);

    axum::Server::bind(&addr)
        .serve(redirect.into_make_service())
        .await
        .unwrap();
}