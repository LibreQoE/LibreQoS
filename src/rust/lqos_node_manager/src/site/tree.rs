use askama::Template;

use axum::{
    Extension,
	extract::{Path,	State},
	http::StatusCode,
    response::IntoResponse,
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
        .route("/:tree_id", get(parent_tree).layer(RequireAuth::login()))
}

async fn parent_tree(
	Path(tree_id): Path<usize>,
	Extension(user): Extension<User>,
	State(state): State<AppState>,
) -> impl IntoResponse {
	let template = TreeTemplate { title: "Network Tree".to_string(), current_user: user, state: state };
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "tree.html")]
struct TreeTemplate {
    title: String,
	current_user: User,
	state: AppState
}