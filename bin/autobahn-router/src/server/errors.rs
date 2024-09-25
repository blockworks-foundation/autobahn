use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_derive::Serialize;

// see https://github.com/tokio-rs/axum/blob/main/examples/error-handling/src/main.rs
// and https://github.com/tokio-rs/axum/blob/main/examples/anyhow-error-response/src/main.rs

pub enum AppError {
    Anyhow(anyhow::Error),
}

struct AppJson<T>(T);

impl<T> IntoResponse for AppJson<T>
where
    axum::Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorResponse {
            message: String,
        }

        let anyhow_message = match self {
            AppError::Anyhow(err) => err.to_string(),
        };

        (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppJson(ErrorResponse {
                message: anyhow_message,
            }),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        AppError::Anyhow(err.into())
    }
}
