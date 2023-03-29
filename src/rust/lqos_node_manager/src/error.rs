use askama::Template;

use axum::{
	http::StatusCode,
	response::{
		Html,
		IntoResponse,
		Redirect,
		Response,
	},
};

#[derive(Debug)]
pub enum AppError {
    InvalidCredentials,
    InternalServerError,
	NotFound,
	Unauthorized,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, content) = match self {
            Self::Unauthorized => {
				return Redirect::to("/auth/login").into_response()
            },
            Self::InternalServerError => {
				let template = Error500 { title: "500 Internal Server Error".to_string() };
				(StatusCode::INTERNAL_SERVER_ERROR, Html(template.render().unwrap()).into_response())
            },
            Self::NotFound => {
				let template = Error404 { title: "404 Not Found".to_string() };
				(StatusCode::NOT_FOUND, Html(template.render().unwrap()).into_response())
            },
            Self::InvalidCredentials => {
				let template = Error404 { title: "400 Bad Request".to_string() };
				(StatusCode::NOT_FOUND, Html(template.render().unwrap()).into_response())
            }
        };
		(status, content).into_response()
    }
}

#[derive(Template)]
#[template(path = "error/404.html")]
struct Error404 {
    title: String,
}

#[derive(Template)]
#[template(path = "error/500.html")]
struct Error500 {
    title: String,
}