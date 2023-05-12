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
    RequestTimeout,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status_code, content) = match self {
            AppError::InternalServerError => {
				let template = Error500 { title: "500 Internal Server Error".to_string() };
				(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html(template.render().unwrap())
                )
            },
            AppError::InvalidCredentials => {
				let template = Error404 { title: "400 Bad Request".to_string() };
				(
                    StatusCode::NOT_FOUND,
                    Html(template.render().unwrap())
                )
            },
            AppError::NotFound => {
				let template = Error404 { title: "404 Not Found".to_string() };
				(
                    StatusCode::NOT_FOUND,
                    Html(template.render().unwrap())
                )
            },
            AppError::RequestTimeout => {
				let template = Error404 { title: "500 Request Timed Out".to_string() };
				(
                    StatusCode::REQUEST_TIMEOUT,
                    Html(template.render().unwrap())
                )
            },
            AppError::Unauthorized => {
				return Redirect::to("/auth/login").into_response()
            },
        };
		(status_code, content).into_response()
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