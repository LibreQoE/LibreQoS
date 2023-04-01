use askama::Template;

use axum::{
    extract::State,
	http::StatusCode,
    response::{Html, IntoResponse, Extension},
    routing::{get, post},
    Form,
	Router,
};

use lqos_bus::IpStats;
use lqos_config::{self, ShapedDevice};

use crate::auth::{self, RequireAuth, Role};
use crate::AppState;

use crate::lqos::tracker::{shaped_devices, unknown_hosts};

pub fn routes() -> Router<AppState> {
    Router::new()
		.route("/dashboard", get(get_dashboard).layer(RequireAuth::login()))
		.route("/devices/add", get(get_add_device).layer(RequireAuth::login_with_role(Role::Admin..)))
		.route("/devices/add", post(post_add_device).layer(RequireAuth::login_with_role(Role::Admin..)))
		.route("/unknown", get(get_unknown_devices).layer(RequireAuth::login()))
		.route("/shaped", get(get_shaped_devices).layer(RequireAuth::login()))
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    title: String,
    current_user: auth::User,
	state: AppState
}

async fn get_dashboard(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let template = DashboardTemplate { title: "Dashboard".to_string(), current_user: user, state: state };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

#[derive(Template)]
#[template(path = "devices/add.html")]
struct AddDeviceTemplate {
    title: String,
    current_user: auth::User,
	state: AppState
}

async fn get_add_device(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let template = AddDeviceTemplate { title: "New Device".to_string(), current_user: user, state: state };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

async fn post_add_device(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let template = AddDeviceTemplate { title: "New Device".to_string(), current_user: user, state: state };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

#[derive(Template)]
#[template(path = "devices/shaped.html")]
struct ShapedDevicesTemplate {
    title: String,
    current_user: auth::User,
	devices: Vec<ShapedDevice>,
	state: AppState
}

async fn get_shaped_devices(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let shaped_devices = shaped_devices().await;
	let template = ShapedDevicesTemplate { title: "Shaped Devices".to_string(), current_user: user, devices: shaped_devices, state: state };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

#[derive(Template)]
#[template(path = "devices/unknown.html")]
struct UnknownDevicesTemplate {
    title: String,
    current_user: auth::User,
	devices: Vec<IpStats>,
	state: AppState
}

async fn get_unknown_devices(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let unknown_devices = unknown_hosts().await;
	let template = UnknownDevicesTemplate { title: "Unknown Devices".to_string(), current_user: user, devices: unknown_devices, state: state };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}