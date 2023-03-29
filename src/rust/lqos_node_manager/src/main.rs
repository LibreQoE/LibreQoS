mod auth;
mod site;
mod lqos;
mod error;

use axum::{
	extract::State,
	http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, get_service},
    Router,
};
use std::{net::SocketAddr, collections::HashSet, sync::{Arc, Mutex}};
use tower::ServiceBuilder;
use tower_http::{
	cors::{Any, CorsLayer},
	services::{ServeDir},
	trace::TraceLayer,
};
use std::time::Duration;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
// Use JemAllocator only on supported platforms
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use jemallocator::Jemalloc;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Clone)]
pub struct AppState {}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lqos_node_manager=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
		
	tokio::spawn(lqos::tracker::update_tracking());
	
	let cors = CorsLayer::new().allow_origin(Any);

	let state = AppState {};

    let app = Router::new()
		.merge(site::main::routes())
		.merge(site::websocket::routes())
		.nest("/tree", site::tree::routes())
		.nest("/circuit", site::circuit::routes())
		.nest("/config", site::config::routes())
		.route_layer(auth::RequireAuth::login())
        .route("/", get(root))
		.nest("/auth", site::auth::routes())
        .nest_service("/assets", get_service(ServeDir::new("assets")))
		.layer(cors)
		.layer(
			ServiceBuilder::new()
				.layer(auth::session_layer())
				.layer(auth::auth_layer())
				.map_response(|r: Response|{
					if r.status()==StatusCode::UNAUTHORIZED {
						error::AppError::Unauthorized.into_response()
					} else if r.status()==StatusCode::NOT_FOUND {
						error::AppError::NotFound.into_response()
					} else if r.status()==StatusCode::INTERNAL_SERVER_ERROR {
						error::AppError::InternalServerError.into_response()
					} else {
						r
					}})
        )
		.layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn root(
	auth: auth::AuthContext,
	State(state): State<AppState>,
) -> impl IntoResponse {
    let user = auth.current_user.clone();
    if user.is_none() {
		if lqos::allow_anonymous() {
			let data = auth::Credentials {
				username: "Anonymous".to_string(),
				password: "".to_string(),
			};
			if auth::authenticate_user(data, auth).await.unwrap() {
				return Redirect::to("/dashboard")
			}
		}
	} else {
		return Redirect::to("/dashboard")
	}
	Redirect::to("/auth/login")
}