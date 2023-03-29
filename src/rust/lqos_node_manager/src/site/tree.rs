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

use lqos_config;
use crate::auth;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/:tree_id", get(parent_tree))
}

async fn parent_tree(
	Path(tree_id): Path<usize>,
	Extension(user): Extension<auth::User>
) -> impl IntoResponse {
	let template = TreeTemplate { title: "Network Tree".to_string(), current_user: user };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

#[derive(Template)]
#[template(path = "tree.html")]
struct TreeTemplate {
    title: String,
	current_user: auth::User,
}