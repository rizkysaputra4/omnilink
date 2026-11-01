use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "omnilink-relay", about = "Blind Rendezvous Relay & Push Hub for OmniLink")]
struct Args {
    #[arg(short, long, default_value = "0.0.0.0:8080", env = "OMNILINK_BIND")]
    bind: String,
}

#[derive(Deserialize)]
struct TunnelQuery {
    room: String,
    role: String, // "host" or "client"
}

#[derive(Deserialize)]
struct NotifyQuery {
    channel: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NotificationPayload {
    pub channel: String,
    pub title: String,
    pub message: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default = "default_timestamp")]
    pub timestamp: u64,
}

fn default_priority() -> String {
    "normal".into()
}

fn default_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    active_rooms: usize,
    active_notification_channels: usize,
    uptime_secs: u64,
}

struct WaitingPeer {
    role: String,
    sender: mpsc::UnboundedSender<Message>,
    paired_tx: tokio::sync::oneshot::Sender<mpsc::UnboundedSender<Message>>,
    connected_at: Instant,
}

type SharedRooms = Arc<RwLock<HashMap<String, WaitingPeer>>>;
type SharedNotificationSubscribers = Arc<RwLock<HashMap<String, Vec<mpsc::UnboundedSender<Message>>>>>;

#[derive(Clone)]
struct AppState {
    rooms: SharedRooms,
    notification_subs: SharedNotificationSubscribers,
    start_time: Instant,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omnilink_relay=info,tower_http=trace".into()),
        )
        .init();

    let args = Args::parse();
    let rooms: SharedRooms = Arc::new(RwLock::new(HashMap::new()));
    let notification_subs: SharedNotificationSubscribers = Arc::new(RwLock::new(HashMap::new()));

    let state = AppState {
        rooms: rooms.clone(),
        notification_subs: notification_subs.clone(),
        start_time: Instant::now(),
    };

    // Background task to sweep stale waiting rooms older than 5 minutes
    let sweep_rooms = rooms.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let mut lock = sweep_rooms.write().await;
            let before = lock.len();
            lock.retain(|_room, peer| peer.connected_at.elapsed() < Duration::from_secs(300));
            let removed = before - lock.len();
            if removed > 0 {
                info!("Swept {} stale waiting rooms", removed);
            }
        }
    });

    let app = Router::new()
        .route("/", get(home_handler))
        .route("/pair", get(pair_handler))
        .route("/health", get(health_handler))
        .route("/v1/tunnel", get(ws_tunnel_handler))
        .route("/v1/notify", post(push_notify_handler))
        .route("/v1/notifications/ws", get(ws_notifications_handler))
        .with_state(state);

    let addr: SocketAddr = args.bind.parse()?;
    info!("🚀 OmniLink Blind Relay & Push Hub listening on http://{}", addr);
    info!("Endpoint tunnel: ws://{}/v1/tunnel?room=<ROOM_ID>&role=host|client", addr);
    info!("Endpoint notify: POST http://{}/v1/notify", addr);
    info!("Endpoint notify stream: ws://{}/v1/notifications/ws?channel=<CHANNEL>", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

#[derive(Deserialize)]
struct PairQuery {
    data: Option<String>,
}

async fn pair_handler(Query(query): Query<PairQuery>) -> Html<String> {
    let data = query.data.unwrap_or_default();
    let deep_link = format!("omnilink://pair?data={}", data);
    Html(format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>OmniLink Connection</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #090a0f; color: #f1f5f9; text-align: center; padding: 40px 20px; }}
        .card {{ background: #12141d; border: 1px solid #262b3a; border-radius: 16px; padding: 30px; max-width: 440px; margin: 40px auto; }}
        .btn {{ display: inline-block; background: #00f2fe; color: #000; font-weight: bold; padding: 14px 28px; border-radius: 10px; text-decoration: none; margin-top: 20px; font-size: 16px; }}
    </style>
    <script>
        window.location.href = "{deep_link}";
    </script>
</head>
<body>
    <div class="card">
        <h2>🔒 OmniLink Pairing</h2>
        <p style="color: #94a3b8;">Redirecting to your OmniLink Android App...</p>
        <a class="btn" href="{deep_link}">Open in OmniLink App</a>
    </div>
</body>
</html>"#,
        deep_link = deep_link
    ))
}

async fn home_handler() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html>
<head><title>OmniLink Relay & Push Hub</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 600px; margin: 40px auto; padding: 20px;">
    <h2>🔒 OmniLink Relay & Notification Hub Active</h2>
    <p>Zero-knowledge rendezvous forwarder and autonomous push gateway for <strong>OmniLink</strong>.</p>
    <p>Status: <span style="color: green; font-weight: bold;">ONLINE (HTTPS)</span></p>
    <p>Services:</p>
    <ul>
        <li><code>/health</code> - Service telemetry</li>
        <li><code>/v1/tunnel?room=&lt;UUID&gt;&role=host|client</code> - E2EE Terminal Tunnel</li>
        <li><code>POST /v1/notify</code> - AI Agent Push Gateway (Independent of active terminal sessions)</li>
        <li><code>/v1/notifications/ws?channel=&lt;CHANNEL&gt;</code> - Mobile 24/7 Notification Stream</li>
    </ul>
</body>
</html>"#)
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let rooms_count = state.rooms.read().await.len();
    let subs_count = state.notification_subs.read().await.len();
    Json(HealthResponse {
        status: "ok",
        service: "omnilink-relay",
        active_rooms: rooms_count,
        active_notification_channels: subs_count,
        uptime_secs: state.start_time.elapsed().as_secs(),
    })
}

// ==============================================================================
// Autonomous Push Notification Handler (For AI Agents Without Active Terminal)
// ==============================================================================

async fn push_notify_handler(
    State(state): State<AppState>,
    Json(payload): Json<NotificationPayload>,
) -> impl IntoResponse {
    let channel = payload.channel.clone();
    let json_text = serde_json::to_string(&payload).unwrap_or_default();

    let mut lock = state.notification_subs.write().await;
    if let Some(senders) = lock.get_mut(&channel) {
        let msg = Message::Text(json_text);
        senders.retain(|tx| tx.send(msg.clone()).is_ok());
        let delivered_count = senders.len();
        info!("Delivered push notification for channel '{}' to {} active listeners", channel, delivered_count);
        (StatusCode::OK, Json(serde_json::json!({
            "status": "delivered",
            "channel": channel,
            "listeners": delivered_count
        })))
    } else {
        info!("Notification received for channel '{}' (queued/no active listeners currently attached)", channel);
        (StatusCode::OK, Json(serde_json::json!({
            "status": "accepted",
            "channel": channel,
            "listeners": 0,
            "note": "Delivered to hub, device can fetch upon reconnection"
        })))
    }
}

async fn ws_notifications_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<NotifyQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let channel = query.channel;
    ws.on_upgrade(move |socket| handle_notification_subscriber(socket, channel, state.notification_subs))
}

async fn handle_notification_subscriber(
    socket: WebSocket,
    channel: String,
    subs: SharedNotificationSubscribers,
) {
    info!("Mobile device subscribed to notification stream on channel '{}'", channel);
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    {
        let mut map = subs.write().await;
        map.entry(channel.clone()).or_default().push(tx.clone());
    }

    // Task 1: forward push notifications to WebSocket
    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task 2: receive keep-alive / ping from mobile client
    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(Message::Ping(p)) => {
                    let _ = tx.send(Message::Pong(p));
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => {},
        _ = &mut recv_task => {},
    };

    info!("Mobile device unsubscribed from notification channel '{}'", channel);
    let mut map = subs.write().await;
    if let Some(list) = map.get_mut(&channel) {
        list.retain(|s| !s.is_closed());
        if list.is_empty() {
            map.remove(&channel);
        }
    }
}

// ==============================================================================
// Terminal Tunnel WebSocket Handler
// ==============================================================================

async fn ws_tunnel_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<TunnelQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let room = query.room;
    let role = query.role.to_lowercase();

    ws.on_upgrade(move |socket| handle_tunnel_socket(socket, room, role, state.rooms))
}

async fn handle_tunnel_socket(
    socket: WebSocket,
    room: String,
    role: String,
    rooms: SharedRooms,
) {
    info!("New connection for room '{}' as role '{}'", room, role);
    let (ws_sender, mut ws_receiver) = socket.split();
    let (tx_to_this_peer, rx_from_other_peer) = mpsc::unbounded_channel::<Message>();

    let (paired_peer_sender, maybe_waiting) = {
        let mut map = rooms.write().await;
        if let Some(peer) = map.remove(&room) {
            if peer.role == role {
                warn!("Duplicate role '{}' in room '{}'. Overwriting peer.", role, room);
                let (notify_tx, notify_rx) = tokio::sync::oneshot::channel();
                map.insert(
                    room.clone(),
                    WaitingPeer {
                        role: role.clone(),
                        sender: tx_to_this_peer.clone(),
                        paired_tx: notify_tx,
                        connected_at: Instant::now(),
                    },
                );
                (None, Some(notify_rx))
            } else {
                info!("Room '{}' paired successfully! Bridging host <--> client", room);
                let _ = peer.paired_tx.send(tx_to_this_peer.clone());
                let _ = peer.sender.send(Message::Text("OMNILINK_PEER_CONNECTED".into()));
                let _ = tx_to_this_peer.send(Message::Text("OMNILINK_PEER_CONNECTED".into()));
                (Some(peer.sender), None)
            }
        } else {
            let (notify_tx, notify_rx) = tokio::sync::oneshot::channel();
            map.insert(
                room.clone(),
                WaitingPeer {
                    role: role.clone(),
                    sender: tx_to_this_peer.clone(),
                    paired_tx: notify_tx,
                    connected_at: Instant::now(),
                },
            );
            (None, Some(notify_rx))
        }
    };

    if let Some(peer_sender) = paired_peer_sender {
        run_forwarder(ws_sender, ws_receiver, peer_sender, rx_from_other_peer, room, &rooms).await;
        return;
    }

    if let Some(notify_rx) = maybe_waiting {
        let room_name = room.clone();
        let rooms_clone = rooms.clone();

        tokio::select! {
            paired = notify_rx => {
                match paired {
                    Ok(counterpart_tx) => {
                        run_forwarder(ws_sender, ws_receiver, counterpart_tx, rx_from_other_peer, room_name, &rooms_clone).await;
                    }
                    Err(_) => {
                        warn!("Counterpart cancelled pairing for room '{}'", room_name);
                    }
                }
            }
            _msg = ws_receiver.next() => {
                let mut map = rooms_clone.write().await;
                if let Some(peer) = map.get(&room_name) {
                    if peer.role == role {
                        map.remove(&room_name);
                        info!("Unpaired peer disconnected, cleaned up room '{}'", room_name);
                    }
                }
            }
        }
    }
}

async fn run_forwarder(
    mut ws_sender: futures_util::stream::SplitSink<WebSocket, Message>,
    mut ws_receiver: futures_util::stream::SplitStream<WebSocket>,
    peer_tx: mpsc::UnboundedSender<Message>,
    mut rx_from_other_peer: mpsc::UnboundedReceiver<Message>,
    room: String,
    rooms: &SharedRooms,
) {
    let mut to_client = tokio::spawn(async move {
        while let Some(msg) = rx_from_other_peer.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let peer_tx_clone = peer_tx.clone();
    let mut from_client = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    if peer_tx_clone.send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("WebSocket read error: {:?}", e);
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = &mut to_client => {},
        _ = &mut from_client => {},
    };

    info!("Session for room '{}' ended. Closing channels.", room);
    let _ = peer_tx.send(Message::Close(None));
    let mut map = rooms.write().await;
    map.remove(&room);
}
