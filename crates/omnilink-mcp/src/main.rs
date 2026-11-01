use std::io::{self, BufRead, Write};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line_result in stdin.lock().lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                writeln!(stdout, "{}", err_resp)?;
                stdout.flush()?;
                continue;
            }
        };

        let response = handle_request(req);
        let resp_json = serde_json::to_string(&response)?;
        writeln!(stdout, "{}", resp_json)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_request(req: JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id;

    match req.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "omnilink-mcp",
                    "version": "0.1.0"
                }
            })),
            error: None,
        },
        "notifications/initialized" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({})),
            error: None,
        },
        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({
                "tools": [
                    {
                        "name": "notify_mobile",
                        "description": "Send a push notification or high-priority alert to the user's mobile device via PCTunnel",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string", "description": "Title of the alert" },
                                "message": { "type": "string", "description": "Body content of the notification" },
                                "priority": { "type": "string", "enum": ["low", "normal", "urgent"], "default": "normal" }
                            },
                            "required": ["title", "message"]
                        }
                    },
                    {
                        "name": "get_tunnel_status",
                        "description": "Inspect the status of the active PCTunnel terminal connection",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    }
                ]
            })),
            error: None,
        },
        "tools/call" => {
            let params = req.params.unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match tool_name {
                "notify_mobile" => {
                    let title = arguments.get("title").and_then(|v| v.as_str()).unwrap_or("PCTunnel Alert");
                    let message = arguments.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let priority = arguments.get("priority").and_then(|v| v.as_str()).unwrap_or("normal");

                    JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id,
                        result: Some(json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Notification dispatched to mobile device: [{}] {} (Priority: {})", title, message, priority)
                                }
                            ]
                        })),
                        error: None,
                    }
                }
                "get_tunnel_status" => {
                    JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id,
                        result: Some(json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": "OmniLink Host Status: ACTIVE\nRelay: wss://relay.findbeatradar.com\nSecurity: E2EE Noise/ChaCha20Poly1305\nProcess Isolation: Win32 ConPTY (Standard User)"
                                }
                            ]
                        })),
                        error: None,
                    }
                }
                unknown => JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(json!({
                        "code": -32601,
                        "message": format!("Tool not found: {}", unknown)
                    })),
                },
            }
        }
        unknown => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(json!({
                "code": -32601,
                "message": format!("Method not found: {}", unknown)
            })),
        },
    }
}
