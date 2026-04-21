//! Provider abstraction for LLM API clients (Anthropic, OpenAI).
//!
//! # Architecture
//!
//! The `Provider` trait defines the contract for sending messages to LLMs.
//! Each provider implementation handles HTTP communication, authentication,
//! streaming (SSE), tool-calling, and token usage extraction.
//!
//! This crate is the first Rust-native runtime dependency in the R1 migration
//! program. It replaces `lib/api-client.js` as the long-term owner of provider
//! integration.

pub mod anthropic;
pub mod model_info;
pub mod model_quirks;
pub mod openai;
pub mod retry;
pub mod types;

use async_trait::async_trait;
use futures::stream::BoxStream;
use types::{MessageRequest, MessageResponse, ModelInfo, StreamEvent};

/// Errors from provider operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("SSE parse error: {0}")]
    SseParse(String),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Timeout after {0}s")]
    Timeout(u64),

    #[error("Max retries ({0}) exhausted")]
    MaxRetries(u32),
}

pub type Result<T> = std::result::Result<T, ProviderError>;

/// A stream of SSE events from a provider.
pub type MessageStream = BoxStream<'static, Result<StreamEvent>>;

/// Provider trait — the core abstraction for LLM API clients.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a non-streaming message and get the complete response.
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse>;

    /// Send a streaming message and get a stream of events.
    async fn stream_message(&self, request: MessageRequest) -> Result<MessageStream>;

    /// Get model metadata (context window, capabilities, etc.).
    fn model_info(&self) -> &ModelInfo;
}
