use askama::Template;

use axum::{
	http::StatusCode,
    extract::{State},
    response::{IntoResponse, Redirect},
    routing::{get},
	Form,
	Router,
};
use crate::auth::{self, authenticate_user, RequireAuth, AuthContext, Credentials, User, Role};
use crate::AppState;
use crate::utils::HtmlTemplate;

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
	auth: AuthContext,
) -> impl IntoResponse {
    if auth.current_user.is_some() {
        Redirect::to("/dashboard").into_response()
	} else {
		let template = LoginTemplate { title: "Login".to_string() };
        HtmlTemplate(template).into_response()
	}
}

pub async fn post_login_handler(
	mut auth: AuthContext,
	Form(data): Form<Credentials>,
) -> impl IntoResponse {
    if authenticate_user(data, auth).await.unwrap() {
        Redirect::to("/dashboard").into_response()
    } else {
		let template = LoginTemplate { title: "Login".to_string() };
        HtmlTemplate(template).into_response()
	}
}

pub async fn logout_handler(
	mut auth: AuthContext,
	State(state): State<AppState>,
) -> impl IntoResponse {
    let user = auth.current_user.clone();
    if let Some(user) = user {
		auth.logout().await;
		tracing::debug!("User logged out: {:?}", user.username);
	}
    Redirect::to("/auth/login").into_response()
}

pub async fn change_password(
	auth: AuthContext,
	State(state): State<AppState>,
) {

}