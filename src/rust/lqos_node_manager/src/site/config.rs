use askama::Template;

use axum::{
    Extension,
	http::StatusCode,
    response::IntoResponse,
    extract::{Path,	State},
    routing::get,
    Form,
	Router,
};
use std::sync::Arc;
use crate::auth::{self, RequireAuth, AuthContext, Credentials, User, Role};
use crate::AppState;
use crate::utils::HtmlTemplate;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/crm", get(get_crm).post(post_crm).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/network", get(get_network).post(post_network).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/server", get(get_server).post(post_server).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/setup", get(get_setup))
        .route("/shaper", get(get_shaper).post(post_shaper).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/tuning", get(get_tuning).post(post_tuning).layer(RequireAuth::login_with_role(Role::Admin..)))
        .route("/users", get(get_users).post(post_users).layer(RequireAuth::login_with_role(Role::Admin..)))
}

#[derive(Template)]
#[template(path = "config/crm.html")]
struct CrmTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_crm(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = CrmTemplate { title: "Configure CRM".to_string(), current_user: user, state: state};
    HtmlTemplate(template)
}

async fn post_crm(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}

#[derive(Template)]
#[template(path = "config/network.html")]
struct NetworkTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_network(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = NetworkTemplate { title: "Configure Network".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_network(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}

#[derive(Template)]
#[template(path = "config/server.html")]
struct ServerTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_server(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = ServerTemplate { title: "Configure Server".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_server(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}

#[derive(Template)]
#[template(path = "config/setup.html")]
struct SetupTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_setup(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = SetupTemplate { title: "Setup Wizard".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "config/shaper.html")]
struct ShaperTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_shaper(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = ShaperTemplate { title: "Configure Shaper".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_shaper(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}

#[derive(Template)]
#[template(path = "config/tuning.html")]
struct TuningTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_tuning(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = TuningTemplate { title: "Configure Tuning".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_tuning(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}

#[derive(Template)]
#[template(path = "config/users/index.html")]
struct UsersTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_users(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
    let template = UsersTemplate { title: "Users".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_users(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}

#[derive(Template)]
#[template(path = "config/users/edit.html")]
struct UserTemplate {
    title: String,
	current_user: User,
	state: AppState
}

async fn get_user(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
    let template = UserTemplate { title: "User".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

async fn post_user(
	Extension(user): Extension<User>,
	State(state): State<AppState>,
	Form(data): Form<Credentials>
) {}