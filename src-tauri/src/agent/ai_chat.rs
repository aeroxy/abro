use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use ai_gateway::vertex::VertexProvider;
use ai_gateway::{ChatProvider, CompletionRequest, StreamEvent, Content, Part, Tool, FunctionDeclaration, FunctionResponse};

use crate::agent::history::{self, HistoryBlock};
use crate::settings::config::ConfigState;

#[derive(Clone, Serialize, Deserialize)]
pub struct AiResponseChunk {
    pub session_id: String,
    pub block_id: String,
    pub delta: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AiResponseDone {
    pub session_id: String,
    pub block_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AiResponseError {
    pub session_id: String,
    pub block_id: String,
    pub error: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AiToolCallEvent {
    pub session_id: String,
    pub block_id: String,
    pub tool_call: ai_gateway::FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub stdout: String,
    pub stderr: String,
    #[serde(rename = "exitCode")]
    pub exit_code: i32,
}

#[tauri::command]
pub fn send_ai_message(
    app: AppHandle,
    state: State<'_, ConfigState>,
    session_id: String,
    block_id: String,
    prompt: String,
    history: Option<Vec<HistoryBlock>>,
) -> Result<(), String> {
    let config = state.0.lock().map_err(|e| e.to_string())?;
    let ai_config = config
        .ai
        .as_ref()
        .ok_or("AI is not configured. Open Settings to configure a provider.")?;

    let provider = VertexProvider::new(
        ai_config.project_id.clone(),
        ai_config.location.clone(),
        ai_config.model.clone(),
        ai_config.credentials_path.clone(),
    );

    // Build contents from history
    let mut contents = if let Some(hist) = history {
        history::build_contents(hist)
    } else {
        Vec::new()
    };

    // Add current user prompt
    contents.push(Content {
        role: "user".to_string(),
        parts: vec![Part::Text { text: prompt }],
    });

    // Define run_terminal_command tool
    let tools = vec![Tool {
        function_declarations: vec![FunctionDeclaration {
            name: "run_terminal_command".to_string(),
            description: "Execute a terminal command. Use when user asks you to run commands or perform system tasks. Always explain what the command will do before proposing it.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute (e.g., 'ls -la', 'git status')"
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Brief explanation of what this command does and why you're running it"
                    }
                },
                "required": ["command", "explanation"]
            }),
        }],
    }];

    // System prompt with guidelines
    let system_prompt = "You are an AI assistant integrated into Abro, a modern terminal application. You can help users with terminal commands, system tasks, and technical questions.

## Available Tools

You have access to a terminal command execution tool. Use it when:
- User explicitly asks you to run a command
- User requests a task that requires shell commands (e.g., \"check disk space\", \"find large files\")
- You need to inspect the system to answer a question

## Guidelines

1. **Always explain first**: Before running commands, briefly explain what you'll do and why
2. **Be cautious**: Don't run destructive commands without clear user intent (rm -rf, mkfs, dd, etc.)
3. **Prefer safe commands**: Use read-only commands when possible (ls, cat, grep vs rm, mv)
4. **One step at a time**: Propose one command, wait for result, then proceed
5. **Learn from feedback**: If user rejects a command, understand why and adjust approach

## Context

You can see the user's recent terminal history including:
- Commands they've run
- Command outputs
- Working directory changes
- Previous AI interactions

Use this context to provide relevant, informed assistance.".to_string();

    let rx = provider
        .stream_complete(&CompletionRequest {
            system_prompt,
            contents: Some(contents),
            user_prompt: None,
            tools: Some(tools),
        })
        .map_err(|e| e.to_string())?;

    // Spawn a thread to read stream events and emit Tauri events
    std::thread::spawn(move || {
        for event in rx {
            match event {
                StreamEvent::Delta(delta) => {
                    let _ = app.emit(
                        "ai-response-chunk",
                        AiResponseChunk {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                            delta,
                        },
                    );
                }
                StreamEvent::FunctionCall(fc) => {
                    let _ = app.emit(
                        "ai-tool-call",
                        AiToolCallEvent {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                            tool_call: fc,
                        },
                    );
                }
                StreamEvent::Done => {
                    let _ = app.emit(
                        "ai-response-done",
                        AiResponseDone {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                        },
                    );
                    return;
                }
                StreamEvent::Error(error) => {
                    let _ = app.emit(
                        "ai-response-error",
                        AiResponseError {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                            error,
                        },
                    );
                    return;
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn continue_ai_with_tool_result(
    app: AppHandle,
    state: State<'_, ConfigState>,
    session_id: String,
    block_id: String,
    tool_name: String,
    tool_result: ToolResult,
    history: Option<Vec<HistoryBlock>>,
) -> Result<(), String> {
    let config = state.0.lock().map_err(|e| e.to_string())?;
    let ai_config = config
        .ai
        .as_ref()
        .ok_or("AI is not configured. Open Settings to configure a provider.")?;

    let provider = VertexProvider::new(
        ai_config.project_id.clone(),
        ai_config.location.clone(),
        ai_config.model.clone(),
        ai_config.credentials_path.clone(),
    );

    // Build contents from history
    let mut contents = if let Some(hist) = history {
        history::build_contents(hist)
    } else {
        Vec::new()
    };

    // Explicitly add the function response (don't rely on history builder alone)
    contents.push(Content {
        role: "function".to_string(),
        parts: vec![Part::FunctionResponse {
            function_response: FunctionResponse {
                name: tool_name,
                response: serde_json::json!({
                    "stdout": tool_result.stdout,
                    "stderr": tool_result.stderr,
                    "exit_code": tool_result.exit_code,
                }),
            },
        }],
    });

    eprintln!("\n=== continue_ai_with_tool_result ===");
    eprintln!("contents count: {}", contents.len());
    for (i, content) in contents.iter().enumerate() {
        eprintln!("  [{}] role={} parts={}", i, content.role, content.parts.len());
    }

    // Define run_terminal_command tool (same as before)
    let tools = vec![Tool {
        function_declarations: vec![FunctionDeclaration {
            name: "run_terminal_command".to_string(),
            description: "Execute a terminal command. Use when user asks you to run commands or perform system tasks. Always explain what the command will do before proposing it.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute (e.g., 'ls -la', 'git status')"
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Brief explanation of what this command does and why you're running it"
                    }
                },
                "required": ["command", "explanation"]
            }),
        }],
    }];

    // System prompt with guidelines
    let system_prompt = "You are an AI assistant integrated into Abro, a modern terminal application. You can help users with terminal commands, system tasks, and technical questions.

## Available Tools

You have access to a terminal command execution tool. Use it when:
- User explicitly asks you to run a command
- User requests a task that requires shell commands (e.g., \"check disk space\", \"find large files\")
- You need to inspect the system to answer a question

## Guidelines

1. **Always explain first**: Before running commands, briefly explain what you'll do and why
2. **Be cautious**: Don't run destructive commands without clear user intent (rm -rf, mkfs, dd, etc.)
3. **Prefer safe commands**: Use read-only commands when possible (ls, cat, grep vs rm, mv)
4. **One step at a time**: Propose one command, wait for result, then proceed
5. **Learn from feedback**: If user rejects a command, understand why and adjust approach

## Context

You can see the user's recent terminal history including:
- Commands they've run
- Command outputs
- Working directory changes
- Previous AI interactions

Use this context to provide relevant, informed assistance.".to_string();

    let rx = provider
        .stream_complete(&CompletionRequest {
            system_prompt,
            contents: Some(contents),
            user_prompt: None,
            tools: Some(tools),
        })
        .map_err(|e| e.to_string())?;

    // Spawn a thread to read stream events and emit Tauri events
    std::thread::spawn(move || {
        for event in rx {
            match event {
                StreamEvent::Delta(delta) => {
                    let _ = app.emit(
                        "ai-response-chunk",
                        AiResponseChunk {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                            delta,
                        },
                    );
                }
                StreamEvent::FunctionCall(fc) => {
                    let _ = app.emit(
                        "ai-tool-call",
                        AiToolCallEvent {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                            tool_call: fc,
                        },
                    );
                }
                StreamEvent::Done => {
                    let _ = app.emit(
                        "ai-response-done",
                        AiResponseDone {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                        },
                    );
                    return;
                }
                StreamEvent::Error(error) => {
                    let _ = app.emit(
                        "ai-response-error",
                        AiResponseError {
                            session_id: session_id.clone(),
                            block_id: block_id.clone(),
                            error,
                        },
                    );
                    return;
                }
            }
        }
    });

    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[tauri::command]
pub fn execute_tool_command(
    command: String,
    cwd: Option<String>,
) -> Result<CommandOutput, String> {
    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let mut cmd = std::process::Command::new(shell);
    cmd.arg(flag).arg(&command);

    if let Some(dir) = &cwd {
        // Expand ~ to home directory
        let expanded = if dir.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                dir.replacen('~', &home.to_string_lossy(), 1)
            } else {
                dir.clone()
            }
        } else {
            dir.clone()
        };
        cmd.current_dir(&expanded);
    }

    match cmd.output() {
        Ok(output) => Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        }),
        Err(e) => Err(format!("Failed to execute command: {}", e)),
    }
}

#[tauri::command]
pub fn list_vertex_models(
    project_id: String,
    location: String,
    credentials_path: Option<String>,
) -> Result<Vec<String>, String> {
    let provider = VertexProvider::new(
        project_id,
        location,
        String::new(), // model not needed for listing
        credentials_path,
    );

    provider.list_models().map_err(|e| e.to_string())
}
