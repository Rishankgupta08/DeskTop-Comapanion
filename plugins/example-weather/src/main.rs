//! # Example Weather Plugin
//!
//! Demonstrates the OpenMate JSON-RPC 2.0 tool execution interface over stdin/stdout. [DR-039]

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: u64,
    method: String,
    params: ToolCallParams,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    tool: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse<T> {
    jsonrpc: &'static str,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Serialize)]
struct ToolCallResult {
    output: String,
    success: bool,
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp: JsonRpcResponse<ToolCallResult> = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: 0,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                    }),
                };
                let _ = writeln!(stdout, "{}", serde_json::to_string(&err_resp).unwrap());
                let _ = stdout.flush();
                continue;
            }
        };

        if req.method == "tool_call" {
            let tool_name = req.params.tool.as_str();
            let city = req
                .params
                .arguments
                .get("city")
                .and_then(|v| v.as_str())
                .unwrap_or("your area");

            let output = match tool_name {
                "get_weather" => {
                    format!("Weather forecast for {}: Currently 32°C (90°F), clear skies and sunny with mild breeze.", city)
                }
                other => {
                    let err_resp: JsonRpcResponse<ToolCallResult> = JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32601,
                            message: format!("Method/Tool not found: {}", other),
                        }),
                    };
                    let _ = writeln!(stdout, "{}", serde_json::to_string(&err_resp).unwrap());
                    let _ = stdout.flush();
                    continue;
                }
            };

            let resp = JsonRpcResponse {
                jsonrpc: "2.0",
                id: req.id,
                result: Some(ToolCallResult {
                    output,
                    success: true,
                }),
                error: None,
            };

            let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
            let _ = stdout.flush();
        }
    }
}
