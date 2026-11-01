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
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "omnilink-relay", about = "Blind Rendezvous Relay for OmniLink")]
struct Args {
    #[arg(short, long, default_value = "0.0.0.0:8080", env = "OMNILINK_BIND")]
    bind: String,
}

#[derive(Deserialize)]
struct TunnelQuery {
    room: String,
    role: String, // "host" or "client"
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    active_rooms: usize,
    uptime_secs: u64,
}

struct WaitingPeer {
    role: String,
    sender: mpsc::UnboundedSender<Message>,
    connected_at: Instant,
}

type SharedRooms = Arc<RwLock<HashMap<String, WaitingPeer>>>;

#[derive(Clone)]
struct AppState {
    rooms: SharedRooms,
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
    let state = AppState {
        rooms: rooms.clone(),
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
        .route("/health", get(health_handler))
        .route("/v1/tunnel", get(ws_tunnel_handler))
        .with_state(state);

    let addr: SocketAddr = args.bind.parse()?;
    info!("🚀 OmniLink Blind Relay listening on http://{}", addr);
    info!("Endpoint tunnel: ws://{}/v1/tunnel?room=<ROOM_ID>&role=host|client", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

async fn home_handler() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html>
<head><title>OmniLink Relay Service</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 600px; margin: 40px auto; padding: 20px;">
    <h2>🔒 OmniLink Blind Relay Active</h2>
    <p>This is a zero-knowledge rendezvous forwarder for <strong>OmniLink</strong>.</p>
    <p>Status: <span style="color: green; font-weight: bold;">ONLINE</span></p>
    <p>Endpoints:</p>
    <ul>
        <li><code>/health</code> - Service telemetry</li>
        <li><code>/v1/tunnel?room=&lt;UUID&gt;&role=host|client</code> - End-to-end WebSocket tunnel</li>
    </ul>
</body>
</html>"#)
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let rooms_count = state.rooms.read().await.len();
    Json(HealthResponse {
        status: "ok",
        service: "omnilink-relay",
        active_rooms: rooms_count,
        uptime_secs: state.start_time.elapsed().as_secs(),
    })
}

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
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Create an unbounded channel for this connection
    let (tx_to_this_peer, mut rx_from_other_peer) = mpsc::unbounded_channel::<Message>();

    // Check if counterpart is already waiting
    let counterpart_tx = {
        let mut map = rooms.write().await;
        if let Some(peer) = map.remove(&room) {
            if peer.role == role {
                // Same role already waiting, replace it
                warn!("Duplicate role '{}' in room '{}'. Overwriting peer.", role, room);
                map.insert(
                    room.clone(),
                    WaitingPeer {
                        role: role.clone(),
                        sender: tx_to_this_peer.clone(),
                        connected_at: Instant::now(),
                    },
                );
                None
            } else {
                // We found our counterpart! (e.g. host meets client or vice versa)
                info!("Room '{}' paired successfully! Bridging host <--> client", room);
                Some(peer.sender)
            }
        } else {
            // No peer waiting yet, register this peer in room
            map.insert(
                room.clone(),
                WaitingPeer {
                    role: role.clone(),
                    sender: tx_to_this_peer.clone(),
                    connected_at: Instant::now(),
                },
            );
            None
        }
    };

    if let Some(peer_tx) = counterpart_tx {
        // We are paired immediately! Notify peer that counterpart has connected
        let _ = peer_tx.send(Message::Text("OMNILINK_PEER_CONNECTED".into()));
        let _ = tx_to_this_peer.send(Message::Text("OMNILINK_PEER_CONNECTED".into()));

        // Run bidirectional forwarder
        run_forwarder(ws_sender, ws_receiver, peer_tx, rx_from_other_peer, room, &rooms).await;
    } else {
        // Wait until counterpart arrives or we get cancelled
        // Spawn task to deliver messages from rx_from_other_peer to ws_sender
        let room_name = room.clone();
        let rooms_clone = rooms.clone();

        let mut send_task = tokio::spawn(async move {
            while let Some(msg) = rx_from_other_peer.recv().await {
                if ws_sender.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Wait on incoming messages until paired or disconnected
        let mut recv_task = tokio::spawn(async move {
            while let Some(msg) = ws_receiver.next().await {
                match msg {
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(Message::Ping(p)) => {
                        let _ = tx_to_this_peer.send(Message::Pong(p));
                    }
                    Ok(_) => {
                        // While waiting, ignore other payloads
                    }
                }
            }
        });

        tokio::select! {
            _ = &mut send_task => {},
            _ = &mut recv_task => {},
        };

        // Clean up room if this peer was still waiting
        let mut map = rooms_clone.write().await;
        if let Some(peer) = map.get(&room_name) {
            if peer.role == role {
                map.remove(&room_name);
                info!("Unpaired peer disconnected, cleaned up room '{}'", room_name);
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
    // Task 1: Forward messages received from other peer to this WebSocket
    let mut to_client = tokio::spawn(async move {
        while let Some(msg) = rx_from_other_peer.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task 2: Forward messages received from this WebSocket to the other peer
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
