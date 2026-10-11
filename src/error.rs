//! The one error type crossing every boundary. Each variant maps to one HTTP
//! status, and the client sees the category and detail string, never an
//! internal path or identifier: an error body is attacker-readable.

use axum::{
    Json,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use thiserror::Error;

/// Every variant is constructed somewhere. `RateLimited`, `ConnectionError` and
/// `RoomError` were only built by their own tests and were deleted; add a
/// variant when a real call site returns it.
#[derive(Error, Debug)]
pub enum ChatError {
    #[error("Room is full")]
    RoomFull,
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    #[error("Resource limit exceeded: {0}")]
    ResourceLimit(String),
    #[error("Security error: {0}")]
    SecurityError(String),
    #[error("Rate limit exceeded: {0}")]
    RateLimitError(String),
}

impl ChatError {
    /// The status this error is reported as. Split out from [`IntoResponse`] so
    /// the mapping is directly assertable in tests without building a response.
    pub fn status(&self) -> StatusCode {
        match self {
            ChatError::RoomFull | ChatError::ResourceLimit(_) => StatusCode::SERVICE_UNAVAILABLE,
            ChatError::RateLimitError(_) => StatusCode::TOO_MANY_REQUESTS,
            ChatError::InvalidMessage(_) => StatusCode::BAD_REQUEST,
            ChatError::SecurityError(_) => StatusCode::FORBIDDEN,
        }
    }

    /// Category shown to the client. Deliberately coarse.
    pub(crate) fn public_message(&self) -> &'static str {
        match self {
            ChatError::RoomFull => "Room is full",
            ChatError::RateLimitError(_) => "Rate limit exceeded",
            ChatError::InvalidMessage(_) => "Invalid message",
            ChatError::ResourceLimit(_) => "Server at capacity",
            ChatError::SecurityError(_) => "Access denied",
        }
    }
}

impl IntoResponse for ChatError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": self.public_message(),
            "details": self.to_string(),
        });

        (self.status(), Json(body)).into_response()
    }
}

pub(crate) type ChatResult<T> = Result<T, ChatError>;
