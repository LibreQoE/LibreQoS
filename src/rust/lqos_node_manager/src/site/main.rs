use askama::Template;

use axum::{
	http::StatusCode,
    response::{Html, IntoResponse, Extension},
    routing::{get, post},
    extract::{
		ConnectInfo,
		State,
		ws::{Message, WebSocket, WebSocketUpgrade},
	},
    Router,
	TypedHeader,
};

use lqos_bus::{IpStats, TcHandle};
use lqos_config::{self, ShapedDevice};

use futures_util::SinkExt;
use futures_util::StreamExt;

use serde_json::{Result, Value, json};

use std::net::Ipv4Addr;

use crate::auth;
use crate::AppState;

use crate::lqos::tracker::{
	current_throughput, throughput_ring, cpu_usage, ram_usage, top_10_downloaders, worst_10_rtt, 
	rtt_histogram, shaped_devices, unknown_hosts, shaped_devices_count, unknown_hosts_count
};

pub fn routes() -> Router<AppState> {
    Router::new()
		.route("/dashboard", get(get_dashboard))
		.route("/devices/add", get(get_add_device).layer(auth::RequireAuth::login()))
		.route("/devices/add", post(post_add_device).layer(auth::RequireAuth::login()))
		.route("/unknown", get(get_unknown_devices))
		.route("/shaped", get(get_shaped_devices))
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    title: String,
    current_user: auth::User,
}

async fn get_dashboard(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let template = DashboardTemplate { title: "Dashboard".to_string(), current_user: user };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

#[derive(Template)]
#[template(path = "devices/add.html")]
struct AddDeviceTemplate {
    title: String,
    current_user: auth::User,
}

async fn get_add_device(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let template = AddDeviceTemplate { title: "New Device".to_string(), current_user: user };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

async fn post_add_device(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = AddDeviceTemplate { title: "New Device".to_string(), current_user: user };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

#[derive(Template)]
#[template(path = "devices/shaped.html")]
struct ShapedDevicesTemplate {
    title: String,
    current_user: auth::User,
	devices: Vec<ShapedDevice>,
}

async fn get_shaped_devices(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let shaped_devices = shaped_devices().await;
	let template = ShapedDevicesTemplate { title: "Shaped Devices".to_string(), current_user: user, devices: shaped_devices };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}

#[derive(Template)]
#[template(path = "devices/unknown.html")]
struct UnknownDevicesTemplate {
    title: String,
    current_user: auth::User,
	devices: Vec<IpStats>,
}

async fn get_unknown_devices(
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let unknown_devices = unknown_hosts().await;
	let template = UnknownDevicesTemplate { title: "Unknown Devices".to_string(), current_user: user, devices: unknown_devices };
    (StatusCode::OK, Html(template.render().unwrap()).into_response())
}