pub mod openrouter;
pub mod vertex;

use std::sync::mpsc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub system_prompt: String,

    // NEW: For multi-turn conversations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<Vec<Content>>,

    // DEPRECATED: Use contents instead (kept for backward compat)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,

    // NEW: Tool definitions for function calling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    pub role: String, // "user" | "model" | "function"
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text { text: String },
    FunctionCall { function_call: FunctionCall },
    FunctionResponse { function_response: FunctionResponse },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    pub name: String,
    pub response: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_name: String,
    pub model: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    Delta(String),
    FunctionCall(FunctionCall),
    Done,
    Error(String),
}

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("provider returned an empty response")]
    EmptyResponse,
    #[error("authentication error: {0}")]
    AuthError(String),
    #[error("HTTP error: {0}")]
    HttpError(String),
    #[error("parse error: {0}")]
    ParseError(String),
}

pub trait ChatProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, GatewayError>;

    fn stream_complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<mpsc::Receiver<StreamEvent>, GatewayError>;
}

#[derive(Debug, Default)]
pub struct MockProvider;

impl ChatProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, GatewayError> {
        let prompt = request.user_prompt.as_deref().unwrap_or("");
        if prompt.trim().is_empty() {
            return Err(GatewayError::EmptyResponse);
        }

        Ok(CompletionResponse {
            content: format!("mock-response: {}", prompt),
        })
    }

    fn stream_complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<mpsc::Receiver<StreamEvent>, GatewayError> {
        let prompt = request.user_prompt.as_deref().unwrap_or("");
        if prompt.trim().is_empty() {
            return Err(GatewayError::EmptyResponse);
        }

        let (tx, rx) = mpsc::channel();
        let content = format!("mock-response: {}", prompt);
        std::thread::spawn(move || {
            let _ = tx.send(StreamEvent::Delta(content));
            let _ = tx.send(StreamEvent::Done);
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_provider_returns_content() {
        let provider = MockProvider;
        let request = CompletionRequest {
            system_prompt: "you are concise".into(),
            user_prompt: Some("list files".into()),
            contents: None,
            tools: None,
        };

        let response = provider.complete(&request).expect("request should succeed");

        assert_eq!(response.content, "mock-response: list files");
    }
}
