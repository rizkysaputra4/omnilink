use std::io::{self, BufRead, Write};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "omnilink-mcp",
    about = "OmniLink Model Context Protocol (MCP) Server - Autonomous Push Notification Gateway"
)]
struct Args {
    /// Relay notification endpoint URL
    #[arg(
        long,
        default_value = "https://relay.findbeatradar.com/v1/notify",
        env = "OMNILINK_NOTIFY_URL"
    )]
    notify_url: String,

    /// Default notification channel / topic
    #[arg(
        long,
        default_value = "default",
        env = "OMNILINK_CHANNEL"
    )]
    channel: String,
}

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

fn load_host_room_id() -> Option<String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let path = std::path::PathBuf::from(home).join(".omnilink").join("host_config.json");
    let content = std::fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&content).ok()?;
    json.get("room_id").and_then(|v| v.as_str()).map(|s| s.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

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

        let response = handle_request(req, &http_client, &args).await;
        let resp_json = serde_json::to_string(&response)?;
        writeln!(stdout, "{}", resp_json)?;
        stdout.flush()?;
    }

    Ok(())
}

async fn handle_request(
    req: JsonRpcRequest,
    client: &reqwest::Client,
    args: &Args,
) -> JsonRpcResponse {
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
                        "name": "notify_user",
                        "description": "Send a push notification or high-priority alert directly to the user's mobile device via OmniLink Push Hub. When tapped on phone, directly opens the associated terminal session.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string", "description": "Title of the alert" },
                                "message": { "type": "string", "description": "Body content of the notification" },
                                "priority": { "type": "string", "enum": ["low", "normal", "urgent"], "default": "normal" },
                                "term_id": { "type": "integer", "description": "Terminal session ID to open upon clicking notification (defaults to 0)" },
                                "channel": { "type": "string", "description": "Optional custom channel/room ID (defaults to persistent room from ~/.omnilink/host_config.json)" }
                            },
                            "required": ["title", "message"]
                        }
                    },
                    {
                        "name": "get_service_status",
                        "description": "Inspect the status of the OmniLink Relay and Push Notification Hub",
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
                "notify_user" => {
                    let title = arguments.get("title").and_then(|v| v.as_str()).unwrap_or("OmniLink Alert");
                    let message = arguments.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let priority = arguments.get("priority").and_then(|v| v.as_str()).unwrap_or("normal");
                    let term_id = arguments.get("term_id").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(0);

                    let host_room = load_host_room_id().unwrap_or_else(|| args.channel.clone());
                    let channel = arguments
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .filter(|c| *c != "default" && !c.is_empty())
                        .unwrap_or(&host_room);

                    let payload = json!({
                        "channel": channel,
                        "title": title,
                        "message": message,
                        "priority": priority,
                        "term_id": term_id,
                    });

                    match client.post(&args.notify_url).json(&payload).send().await {
                        Ok(res) if res.status().is_success() => {
                            let body = res.text().await.unwrap_or_default();
                            JsonRpcResponse {
                                jsonrpc: "2.0".into(),
                                id,
                                result: Some(json!({
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": format!("✅ Push notification successfully delivered to OmniLink Hub:\nTitle: {}\nMessage: {}\nPriority: {}\nTerm ID: {}\nChannel: {}\nHub Response: {}", title, message, priority, term_id, channel, body)
                                        }
                                    ]
                                })),
                                error: None,
                            }
                        }
                        Ok(res) => {
                            JsonRpcResponse {
                                jsonrpc: "2.0".into(),
                                id,
                                result: Some(json!({
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": format!("⚠️ Notification sent but relay returned status {}: check channel configuration.", res.status())
                                        }
                                    ]
                                })),
                                error: None,
                            }
                        }
                        Err(e) => {
                            JsonRpcResponse {
                                jsonrpc: "2.0".into(),
                                id,
                                result: Some(json!({
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": format!("❌ Failed to connect to OmniLink Push Hub at {}: {:?}", args.notify_url, e)
                                        }
                                    ]
                                })),
                                error: None,
                            }
                        }
                    }
                }
                "get_service_status" => {
                    let health_url = "https://relay.findbeatradar.com/health";
                    let status_text = match client.get(health_url).send().await {
                        Ok(r) if r.status().is_success() => {
                            let text = r.text().await.unwrap_or_default();
                            format!("OmniLink Relay Status: ONLINE (HTTPS Active)\nEndpoint: {}\nHealth: {}", health_url, text)
                        }
                        _ => "OmniLink Relay Status: UNREACHABLE or OFFLINE".to_string(),
                    };

                    JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id,
                        result: Some(json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": status_text
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
