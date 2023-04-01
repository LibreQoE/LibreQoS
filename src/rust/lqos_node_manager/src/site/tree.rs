use askama::Template;

use axum::{
	extract::{Path,	State},
	http::StatusCode,
    response::{Html, IntoResponse, Extension},
    routing::get,
    Form,
	Router,
};

use crate::auth::{self, RequireAuth};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/:tree_id", get(parent_tree).layer(RequireAuth::login()))
}

async fn parent_tree(
	Path(tree_id): Path<usize>,
	Extension(user): Extension<auth::User>,
	State(state): State<AppState>
) -> impl IntoResponse {
	let template = TreeTemplate { title: "Network Tree".to_string(), current_user: user, state: state };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

#[derive(Template)]
#[template(path = "tree.html")]
struct TreeTemplate {
    title: String,
	current_user: auth::User,
	state: AppState
}