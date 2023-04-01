use askama::Template;

use axum::{
	http::StatusCode,
    extract::{
		Path,
		State,
    },
    response::{Html, IntoResponse, Extension},
    routing::get,
    Router,
};

use crate::auth::{self, RequireAuth, Role};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/crm", get(get_crm).post(post_crm).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/network", get(get_network).post(post_network).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/server", get(get_server).post(post_server).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/shaper", get(get_shaper).post(post_shaper).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/tuning", get(get_tuning).post(post_tuning).layer(RequireAuth::login_with_role(Role::Admin..)))
}

#[derive(Template)]
#[template(path = "config/crm.html")]
struct CrmTemplate {
    title: String,
	current_user: auth::User
}

async fn get_crm(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = CrmTemplate { title: "Configure CRM".to_string(), current_user: user };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

async fn post_crm(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
    (StatusCode::OK, Html("").into_response())
}

#[derive(Template)]
#[template(path = "config/network.html")]
struct NetworkTemplate {
    title: String,
	current_user: auth::User
}

async fn get_network(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = NetworkTemplate { title: "Configure Network".to_string(), current_user: user };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

async fn post_network(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
    (StatusCode::OK, Html("").into_response())
}

#[derive(Template)]
#[template(path = "config/server.html")]
struct ServerTemplate {
    title: String,
	current_user: auth::User
}

async fn get_server(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = ServerTemplate { title: "Configure Server".to_string(), current_user: user };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

async fn post_server(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
    (StatusCode::OK, Html("").into_response())
}

#[derive(Template)]
#[template(path = "config/shaper.html")]
struct ShaperTemplate {
    title: String,
	current_user: auth::User
}

async fn get_shaper(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = ShaperTemplate { title: "Configure Shaper".to_string(), current_user: user };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

async fn post_shaper(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
    (StatusCode::OK, Html("").into_response())
}

#[derive(Template)]
#[template(path = "config/tuning.html")]
struct TuningTemplate {
    title: String,
	current_user: auth::User
}

async fn get_tuning(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = TuningTemplate { title: "Configure Tuning".to_string(), current_user: user };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

async fn post_tuning(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
    (StatusCode::OK, Html("").into_response())
}

async fn post_users(
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
    (StatusCode::OK, Html("").into_response())
}