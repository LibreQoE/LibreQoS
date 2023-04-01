use askama::Template;

use axum::{
	http::StatusCode,
    extract::{State},
    response::{Html, IntoResponse, Redirect},
    routing::{get},
	Form,
	Router,
};

use crate::auth::{self, RequireAuth};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/change-password", get(change_password).layer(RequireAuth::login()))
        .route("/login", get(get_login_handler).post(post_login_handler))
        .route("/logout", get(logout_handler))
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    title: String,
}

pub async fn get_login_handler(
	auth: auth::AuthContext,
) -> impl IntoResponse {
    let user = auth.current_user.clone();
    if user {
		Redirect::to("/dashboard").into_response()
	} else {
		let template = LoginTemplate { title: "Login".to_string() };
		(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
	}
}

pub async fn post_login_handler(
	auth: auth::AuthContext,
	Form(data): Form<auth::Credentials>,
) -> impl IntoResponse {
	if auth::authenticate_user(data, auth).await.unwrap() {
		Redirect::to("/dashboard").into_response()
	} else {
		let template = LoginTemplate { title: "Login".to_string() };
		(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
	}
}

pub async fn logout_handler(
	mut auth: auth::AuthContext,
	State(state): State<AppState>
) -> Redirect {
    let user = auth.current_user.clone();
    if let Some(user) = user {
		auth.logout().await;
		tracing::debug!("User logged out: {:?}", user.username);
	}
	Redirect::to("/auth/login")
}

pub async fn change_password(
	auth: auth::AuthContext,
	State(state): State<AppState>
) -> impl IntoResponse {
	(StatusCode::OK, Html("")).into_response()
}