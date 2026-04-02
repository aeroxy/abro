use serde::{Deserialize, Serialize};
use ai_gateway::{Content, FunctionCall, Part};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub args: serde_json::Value,
    pub state: String, // "pending" | "approved" | "rejected" | "completed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ToolResultInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultInfo {
    pub stdout: String,
    pub stderr: String,
    #[serde(rename = "exitCode")]
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryBlock {
    pub command: String,
    pub output: String,
    #[serde(rename = "type")]
    pub block_type: String, // "command" | "ai"
    #[serde(skip_serializing_if = "Option::is_none", rename = "toolCall")]
    pub tool_call: Option<ToolCallInfo>,
}

pub fn build_contents(history: Vec<HistoryBlock>) -> Vec<Content> {
    let mut contents = Vec::new();

    // Take last 20 blocks
    for block in history.iter().rev().take(20).rev() {
        match block.block_type.as_str() {
            "command" => {
                // User ran a command
                contents.push(Content {
                    role: "user".to_string(),
                    parts: vec![Part::Text {
                        text: format!("$ {}\n{}", block.command, block.output),
                    }],
                });
            }
            "ai" => {
                // Include the user's prompt (stored in command field)
                if !block.command.is_empty() {
                    contents.push(Content {
                        role: "user".to_string(),
                        parts: vec![Part::Text {
                            text: block.command.clone(),
                        }],
                    });
                }

                if let Some(tc) = &block.tool_call {
                    // Add the model's function call
                    contents.push(Content {
                        role: "model".to_string(),
                        parts: vec![Part::FunctionCall {
                            function_call: FunctionCall {
                                name: tc.name.clone(),
                                args: tc.args.clone(),
                            },
                        }],
                    });
                    // Note: function response is added explicitly by the backend
                    // via continue_ai_with_tool_result, not here (avoids duplicates)

                    // If there was text output after the tool call, add it
                    if !block.output.is_empty() {
                        contents.push(Content {
                            role: "model".to_string(),
                            parts: vec![Part::Text {
                                text: block.output.clone(),
                            }],
                        });
                    }
                } else {
                    // AI responded with text only
                    if !block.output.is_empty() {
                        contents.push(Content {
                            role: "model".to_string(),
                            parts: vec![Part::Text {
                                text: block.output.clone(),
                            }],
                        });
                    }
                }
            }
            _ => {}
        }
    }

    contents
}
