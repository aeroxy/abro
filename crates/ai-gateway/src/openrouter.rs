use std::collections::HashMap;
use std::io::BufRead;
use std::sync::mpsc;

use crate::{
    ChatProvider, CompletionRequest, CompletionResponse, Content, FunctionCall, GatewayError,
    Part, StreamEvent, Tool,
};

pub struct OpenRouterProvider {
    pub api_key: String,
    pub model: String,
    http: reqwest::blocking::Client,
}

impl OpenRouterProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            http: reqwest::blocking::Client::new(),
        }
    }

    fn build_messages(&self, request: &CompletionRequest) -> Vec<serde_json::Value> {
        let mut messages: Vec<serde_json::Value> = Vec::new();

        if !request.system_prompt.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": request.system_prompt,
            }));
        }

        if let Some(contents) = &request.contents {
            for content in contents {
                if let Some(msg) = content_to_openai_message(content) {
                    messages.push(msg);
                }
            }
        } else if let Some(prompt) = &request.user_prompt {
            messages.push(serde_json::json!({
                "role": "user",
                "content": prompt,
            }));
        }

        messages
    }

    fn build_tools(&self, tools: &[Tool]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .flat_map(|t| &t.function_declarations)
            .map(|fd| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": fd.name,
                        "description": fd.description,
                        "parameters": fd.parameters,
                    }
                })
            })
            .collect()
    }

    fn build_body(&self, request: &CompletionRequest) -> serde_json::Value {
        let messages = self.build_messages(request);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });

        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                let openai_tools = self.build_tools(tools);
                body["tools"] = serde_json::Value::Array(openai_tools);
            }
        }

        body
    }
}

fn content_to_openai_message(content: &Content) -> Option<serde_json::Value> {
    match content.role.as_str() {
        "user" => {
            let text = content.parts.iter().find_map(|p| {
                if let Part::Text { text } = p {
                    Some(text.clone())
                } else {
                    None
                }
            })?;
            Some(serde_json::json!({ "role": "user", "content": text }))
        }
        "model" => {
            // Check for function call first
            if let Some(Part::FunctionCall { function_call }) = content.parts.first() {
                let args_str = serde_json::to_string(&function_call.args).unwrap_or_default();
                Some(serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": format!("call_{}", function_call.name),
                        "type": "function",
                        "function": {
                            "name": function_call.name,
                            "arguments": args_str,
                        }
                    }]
                }))
            } else {
                let text = content.parts.iter().find_map(|p| {
                    if let Part::Text { text } = p {
                        Some(text.clone())
                    } else {
                        None
                    }
                });
                Some(serde_json::json!({ "role": "assistant", "content": text.unwrap_or_default() }))
            }
        }
        "function" => {
            if let Some(Part::FunctionResponse { function_response }) = content.parts.first() {
                let content_str =
                    serde_json::to_string(&function_response.response).unwrap_or_default();
                Some(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": format!("call_{}", function_response.name),
                    "content": content_str,
                }))
            } else {
                None
            }
        }
        _ => None,
    }
}

impl OpenRouterProvider {
    pub fn list_models(&self) -> Result<Vec<String>, GatewayError> {
        let resp = self
            .http
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(&self.api_key)
            .send()
            .map_err(|e| GatewayError::HttpError(format!("list models failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            let error_msg = if body.len() > 200 {
                format!("{}...", &body[..200])
            } else {
                body
            };
            return Err(GatewayError::HttpError(format!(
                "list models error ({status}): {error_msg}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .map_err(|e| GatewayError::ParseError(format!("invalid response: {e}")))?;

        let models: Vec<String> = json
            .pointer("/data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.pointer("/id").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }
}

impl ChatProvider for OpenRouterProvider {
    fn name(&self) -> &'static str {
        "openrouter"
    }

    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, GatewayError> {
        let rx = self.stream_complete(request)?;
        let mut content = String::new();
        for event in rx {
            match event {
                StreamEvent::Delta(d) => content.push_str(&d),
                StreamEvent::FunctionCall(_) => {}
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
        let body = self.build_body(request);
        let client = self.http.clone();
        let api_key = self.api_key.clone();

        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let result = client
                .post("https://openrouter.ai/api/v1/chat/completions")
                .bearer_auth(&api_key)
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
                let error_msg = if body.len() > 200 {
                    format!("{}...", &body[..200])
                } else {
                    body
                };
                let _ = tx.send(StreamEvent::Error(format!("API error ({status}): {error_msg}")));
                return;
            }

            // Accumulate tool call chunks: index -> (id, name, arguments_buf)
            let mut tool_calls: HashMap<usize, (String, String, String)> = HashMap::new();

            let reader = std::io::BufReader::new(resp);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = tx.send(StreamEvent::Error(format!("read error: {e}")));
                        return;
                    }
                };

                let Some(json_str) = line.strip_prefix("data: ") else {
                    continue;
                };

                if json_str.trim() == "[DONE]" {
                    // Flush accumulated tool calls before Done
                    let mut indices: Vec<usize> = tool_calls.keys().copied().collect();
                    indices.sort();
                    for idx in indices {
                        if let Some((_, name, args_str)) = tool_calls.remove(&idx) {
                            let args: serde_json::Value =
                                serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Object(Default::default()));
                            let _ = tx.send(StreamEvent::FunctionCall(FunctionCall {
                                name,
                                args,
                            }));
                        }
                    }
                    let _ = tx.send(StreamEvent::Done);
                    return;
                }

                let parsed: serde_json::Value = match serde_json::from_str(json_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Handle tool call chunks
                if let Some(tc_arr) = parsed.pointer("/choices/0/delta/tool_calls").and_then(|v| v.as_array()) {
                    for tc in tc_arr {
                        let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let entry = tool_calls.entry(idx).or_insert_with(|| (String::new(), String::new(), String::new()));

                        if let Some(id) = tc.pointer("/id").and_then(|v| v.as_str()) {
                            entry.0 = id.to_string();
                        }
                        if let Some(name) = tc.pointer("/function/name").and_then(|v| v.as_str()) {
                            entry.1 = name.to_string();
                        }
                        if let Some(args_chunk) = tc.pointer("/function/arguments").and_then(|v| v.as_str()) {
                            entry.2.push_str(args_chunk);
                        }
                    }
                    continue;
                }

                // Handle text delta
                if let Some(text) = parsed
                    .pointer("/choices/0/delta/content")
                    .and_then(|v| v.as_str())
                {
                    if !text.is_empty() && tx.send(StreamEvent::Delta(text.to_string())).is_err() {
                        return;
                    }
                }
            }

            // Stream ended without [DONE] — flush tool calls anyway
            let mut indices: Vec<usize> = tool_calls.keys().copied().collect();
            indices.sort();
            for idx in indices {
                if let Some((_, name, args_str)) = tool_calls.remove(&idx) {
                    let args: serde_json::Value =
                        serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Object(Default::default()));
                    let _ = tx.send(StreamEvent::FunctionCall(FunctionCall { name, args }));
                }
            }
            let _ = tx.send(StreamEvent::Done);
        });

        Ok(rx)
    }
}
