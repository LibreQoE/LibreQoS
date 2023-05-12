use askama::Template;

use axum::{
    Extension,
    extract::State,
	http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Form,
	Router,
};

use lqos_bus::IpStats;
use lqos_config::{self, ShapedDevice};
use std::sync::Arc;
use crate::auth::{self, RequireAuth, AuthContext, Credentials, User, Role};
use crate::AppState;
use crate::utils::HtmlTemplate;

use crate::tracker;

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
    current_user: User,
	state: AppState
}

async fn get_dashboard(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = DashboardTemplate { title: "Dashboard".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "devices/add.html")]
struct AddDeviceTemplate {
    title: String,
    current_user: User,
	state: AppState
}

async fn get_add_device(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = AddDeviceTemplate { title: "New Device".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_add_device(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = AddDeviceTemplate { title: "New Device".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "devices/shaped.html")]
struct ShapedDevicesTemplate {
    title: String,
    current_user: User,
	devices: Vec<Device>,
	state: AppState
}

async fn get_shaped_devices(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let devices = state.tracker.cache_manager.buffers;
	let template = ShapedDevicesTemplate { title: "Shaped Devices".to_string(), current_user: user, devices: devices, state: state };
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "devices/unknown.html")]
struct UnknownDevicesTemplate {
    title: String,
    current_user: User,
	devices: Vec<IpStats>,
	state: AppState
}

async fn get_unknown_devices(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let unknown_devices = lqos::bus::all_unknown_ips().await;
	let template = UnknownDevicesTemplate { title: "Unknown Devices".to_string(), current_user: user, devices: unknown_devices, state: state };
    HtmlTemplate(template)
}