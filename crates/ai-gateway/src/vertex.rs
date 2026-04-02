use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{
    ChatProvider, CompletionRequest, CompletionResponse, GatewayError, StreamEvent,
};

#[derive(Debug, Deserialize)]
struct GcloudCredentials {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

struct CachedToken {
    access_token: String,
    obtained_at: Instant,
    expires_in_secs: u64,
}

impl CachedToken {
    fn is_expired(&self) -> bool {
        self.obtained_at.elapsed().as_secs() + 60 > self.expires_in_secs
    }
}

pub struct VertexProvider {
    pub project_id: String,
    pub location: String,
    pub model: String,
    pub credentials_path: PathBuf,
    token_cache: Mutex<Option<CachedToken>>,
    http: reqwest::blocking::Client,
}

impl VertexProvider {
    pub fn new(
        project_id: String,
        location: String,
        model: String,
        credentials_path: Option<String>,
    ) -> Self {
        let creds_path = credentials_path
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".config/gcloud/application_default_credentials.json")
            });

        Self {
            project_id,
            location,
            model,
            credentials_path: creds_path,
            token_cache: Mutex::new(None),
            http: reqwest::blocking::Client::new(),
        }
    }

    pub fn list_models(&self) -> Result<Vec<String>, crate::GatewayError> {
        let token = self.get_access_token()?;
        let url = format!(
            "https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models",
            location = self.location,
            project = self.project_id,
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .map_err(|e| crate::GatewayError::HttpError(format!("list models failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            // Truncate long HTML error responses
            let error_msg = if body.len() > 200 {
                format!("{}...", &body[..200])
            } else {
                body
            };
            return Err(crate::GatewayError::HttpError(format!(
                "list models error ({status}): {error_msg}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .map_err(|e| crate::GatewayError::ParseError(format!("invalid response: {e}")))?;

        let models: Vec<String> = json
            .pointer("/models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        m.pointer("/name")
                            .and_then(|n| n.as_str())
                            .and_then(|n| n.split('/').last())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }

    fn read_credentials(&self) -> Result<GcloudCredentials, GatewayError> {
        let raw = std::fs::read_to_string(&self.credentials_path).map_err(|e| {
            GatewayError::AuthError(format!(
                "cannot read credentials at {}: {e}",
                self.credentials_path.display()
            ))
        })?;
        serde_json::from_str(&raw).map_err(|e| {
            GatewayError::AuthError(format!("invalid credentials format: {e}"))
        })
    }

    fn update_refresh_token(&self, new_refresh_token: &str) -> Result<(), GatewayError> {
        // Read the entire credentials file as JSON
        let raw = std::fs::read_to_string(&self.credentials_path).map_err(|e| {
            GatewayError::AuthError(format!(
                "cannot read credentials at {}: {e}",
                self.credentials_path.display()
            ))
        })?;

        let mut creds_json: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
            GatewayError::AuthError(format!("invalid credentials format: {e}"))
        })?;

        // Update the refresh_token field
        creds_json["refresh_token"] = serde_json::Value::String(new_refresh_token.to_string());

        // Write back to file with proper formatting
        let updated = serde_json::to_string_pretty(&creds_json).map_err(|e| {
            GatewayError::AuthError(format!("failed to serialize credentials: {e}"))
        })?;

        std::fs::write(&self.credentials_path, updated).map_err(|e| {
            GatewayError::AuthError(format!(
                "failed to write credentials to {}: {e}",
                self.credentials_path.display()
            ))
        })?;

        Ok(())
    }

    fn fetch_access_token(&self) -> Result<(String, u64, Option<String>), GatewayError> {
        let creds = self.read_credentials()?;

        #[derive(Serialize)]
        struct TokenRequest<'a> {
            grant_type: &'a str,
            client_id: &'a str,
            client_secret: &'a str,
            refresh_token: &'a str,
        }

        let resp = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&TokenRequest {
                grant_type: "refresh_token",
                client_id: &creds.client_id,
                client_secret: &creds.client_secret,
                refresh_token: &creds.refresh_token,
            })
            .send()
            .map_err(|e| GatewayError::AuthError(format!("token request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            // Truncate long error responses
            let error_msg = if body.len() > 200 {
                format!("{}...", &body[..200])
            } else {
                body
            };
            return Err(GatewayError::AuthError(format!(
                "token exchange failed ({status}): {error_msg}"
            )));
        }

        let token_resp: TokenResponse = resp
            .json()
            .map_err(|e| GatewayError::AuthError(format!("invalid token response: {e}")))?;

        Ok((
            token_resp.access_token,
            token_resp.expires_in,
            token_resp.refresh_token,
        ))
    }

    fn get_access_token(&self) -> Result<String, GatewayError> {
        let mut cache = self.token_cache.lock().unwrap();
        if let Some(ref cached) = *cache {
            if !cached.is_expired() {
                return Ok(cached.access_token.clone());
            }
        }

        let (token, expires_in, new_refresh_token) = self.fetch_access_token()?;

        // Update the refresh token in the credentials file if a new one was returned
        if let Some(ref new_token) = new_refresh_token {
            // Drop the lock before updating the file to avoid holding it during I/O
            drop(cache);
            self.update_refresh_token(new_token)?;
            cache = self.token_cache.lock().unwrap();
        }

        *cache = Some(CachedToken {
            access_token: token.clone(),
            obtained_at: Instant::now(),
            expires_in_secs: expires_in,
        });
        Ok(token)
    }

    fn api_url(&self) -> String {
        format!(
            "https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:streamGenerateContent?alt=sse",
            location = self.location,
            project = self.project_id,
            model = self.model,
        )
    }

    fn build_body(&self, request: &CompletionRequest) -> serde_json::Value {
        let mut body = serde_json::json!({});

        // System instruction
        if !request.system_prompt.is_empty() {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": request.system_prompt}]
            });
        }

        // Contents (multi-turn or single prompt)
        if let Some(contents) = &request.contents {
            body["contents"] = serde_json::to_value(contents).unwrap();
        } else if let Some(prompt) = &request.user_prompt {
            // Backward compatibility
            body["contents"] = serde_json::json!([{
                "role": "user",
                "parts": [{"text": prompt}]
            }]);
        }

        // Tools (function declarations)
        if let Some(tools) = &request.tools {
            body["tools"] = serde_json::to_value(tools).unwrap();
        }

        body
    }
}

impl ChatProvider for VertexProvider {
    fn name(&self) -> &'static str {
        "vertex"
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, GatewayError> {
        let rx = self.stream_complete(request)?;
        let mut content = String::new();
        for event in rx {
            match event {
                StreamEvent::Delta(d) => content.push_str(&d),
                StreamEvent::FunctionCall(_) => {
                    // Skip function calls in non-streaming mode
                    // (streaming mode is preferred for tool use)
                }
                StreamEvent::Done => break,
                StreamEvent::Error(e) => return Err(GatewayError::HttpError(e)),
            }
        }
        if content.is_empty() {
            return Err(GatewayError::EmptyResponse);
        }
        Ok(CompletionResponse { content })
    }

    fn stream_complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<mpsc::Receiver<StreamEvent>, GatewayError> {
        let token = self.get_access_token()?;
        let url = self.api_url();
        let body = self.build_body(request);
        let client = self.http.clone();

        // Log request details for debugging
        eprintln!("\n=== Vertex AI Request ===");
        eprintln!("URL: {}", url);
        eprintln!("Method: POST");
        eprintln!("Headers:");
        eprintln!("  Authorization: Bearer {}", &token);
        eprintln!("  Content-Type: application/json");
        eprintln!("Payload:\n{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        eprintln!("========================\n");

        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let result = client
                .post(&url)
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .send();

            let resp = match result {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(format!("request failed: {e}")));
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().unwrap_or_default();

                eprintln!("\n=== Vertex AI Response Error ===");
                eprintln!("Status: {}", status);
                eprintln!("Body: {}", body);
                eprintln!("================================\n");

                // Truncate long HTML error responses
                let error_msg = if body.len() > 200 {
                    format!("{}...", &body[..200])
                } else {
                    body
                };
                let _ = tx.send(StreamEvent::Error(format!("API error ({status}): {error_msg}")));
                return;
            }

            let reader = std::io::BufReader::new(resp);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = tx.send(StreamEvent::Error(format!("read error: {e}")));
                        return;
                    }
                };

                // SSE format: lines starting with "data: " contain JSON
                let Some(json_str) = line.strip_prefix("data: ") else {
                    continue;
                };

                let parsed: serde_json::Value = match serde_json::from_str(json_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Check for function call FIRST
                if let Some(fc) = parsed.pointer("/candidates/0/content/parts/0/functionCall") {
                    match serde_json::from_value::<crate::FunctionCall>(fc.clone()) {
                        Ok(function_call) => {
                            if tx.send(StreamEvent::FunctionCall(function_call)).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(StreamEvent::Error(format!("invalid functionCall: {e}")));
                            return;
                        }
                    }
                }
                // Then check for text
                else if let Some(text) = parsed
                    .pointer("/candidates/0/content/parts/0/text")
                    .and_then(|v| v.as_str())
                {
                    if tx.send(StreamEvent::Delta(text.to_string())).is_err() {
                        return;
                    }
                }
            }

            let _ = tx.send(StreamEvent::Done);
        });

        Ok(rx)
    }
}
