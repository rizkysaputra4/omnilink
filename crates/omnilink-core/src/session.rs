use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use std::io::{Read, Write};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use omnilink_proto::{verify_auth_proof, Frame, KeyPair, SessionCrypto};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

pub struct TerminalSession {
    pub term_id: u32,
    pub shell: String,
    pub master: Mutex<Box<dyn MasterPty + Send>>,
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub history: Mutex<Vec<u8>>,
    pub display_text: Mutex<String>,
}

impl TerminalSession {
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let master = self.master.lock().map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

pub type SharedTerminalRegistry = Arc<Mutex<HashMap<u32, Arc<TerminalSession>>>>;
pub type SharedBroadcaster = Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>;

#[derive(Serialize, Deserialize, Default)]
pub struct DeviceWhitelist {
    pub authorized_keys: Vec<AuthorizedDevice>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AuthorizedDevice {
    pub name: String,
    pub public_key_hex: String,
    pub authorized_at: String,
}

impl DeviceWhitelist {
    pub fn load() -> Self {
        if let Some(path) = Self::file_path() {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(data) = serde_json::from_str::<Self>(&content) {
                        return data;
                    }
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        if let Some(path) = Self::file_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let data = serde_json::to_string_pretty(self)?;
            std::fs::write(path, data)?;
        }
        Ok(())
    }

    fn file_path() -> Option<PathBuf> {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()?;
        Some(PathBuf::from(home).join(".omnilink").join("authorized_devices.json"))
    }
}

pub struct TunnelHostSession {
    pub relay_url: String,
    pub room_id: String,
    pub host_key: Arc<KeyPair>,
    pub auth_token: [u8; 32],
    pub pin: String,
    pub shell: String,
    pub killswitch: Arc<AtomicBool>,
    pub terminals: SharedTerminalRegistry,
    pub broadcaster: SharedBroadcaster,
}

impl TunnelHostSession {
    pub fn new(
        relay_url: String,
        room_id: String,
        host_key: Arc<KeyPair>,
        auth_token: [u8; 32],
        pin: String,
        shell: String,
    ) -> (Self, Arc<AtomicBool>) {
        Self::with_terminals(
            relay_url,
            room_id,
            host_key,
            auth_token,
            pin,
            shell,
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(None)),
        )
    }

    pub fn with_terminals(
        relay_url: String,
        room_id: String,
        host_key: Arc<KeyPair>,
        auth_token: [u8; 32],
        pin: String,
        shell: String,
        terminals: SharedTerminalRegistry,
        broadcaster: SharedBroadcaster,
    ) -> (Self, Arc<AtomicBool>) {
        let killswitch = Arc::new(AtomicBool::new(false));
        let session = Self {
            relay_url,
            room_id,
            host_key,
            auth_token,
            pin,
            shell,
            killswitch: killswitch.clone(),
            terminals,
            broadcaster,
        };
        (session, killswitch)
    }

    pub async fn run(&self) -> Result<()> {
        let ws_url = format!("{}/v1/tunnel?room={}&role=host", self.relay_url, self.room_id);
        info!("Connecting to relay: {}", ws_url);

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .context("Failed to connect to relay WebSocket")?;

        info!("Connected to relay. Waiting for authorized mobile device...");

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // 1. Wait for counterpart connection notification from relay
        while let Some(msg_res) = ws_receiver.next().await {
            if self.killswitch.load(Ordering::Relaxed) {
                bail!("Killswitch activated before pairing");
            }
            match msg_res {
                Ok(Message::Text(t)) if t == "OMNILINK_PEER_CONNECTED" => {
                    info!("Incoming peer connection to room! Awaiting cryptographic proof of authorization...");
                    break;
                }
                Ok(Message::Ping(p)) => {
                    let _ = ws_sender.send(Message::Pong(p)).await;
                }
                Ok(Message::Close(_)) => {
                    bail!("Relay closed connection while waiting for mobile peer");
                }
                Err(e) => {
                    bail!("WebSocket error: {:?}", e);
                }
                _ => {}
            }
        }

        // 2. Perform E2EE Handshake & Cryptographic Proof Verification
        let init_msg = ws_receiver
            .next()
            .await
            .context("Stream ended before HandshakeInit")??;

        let raw_bytes = match init_msg {
            Message::Binary(b) => b,
            other => bail!("Expected binary frame for HandshakeInit, got {:?}", other),
        };

        let init_frame = Frame::decode(&raw_bytes)?;
        let (client_ephemeral_pub, client_id, auth_proof) = match init_frame {
            Frame::HandshakeInit {
                client_ephemeral_pub,
                client_id,
                auth_proof,
            } => (client_ephemeral_pub, client_id, auth_proof),
            other => bail!("Expected HandshakeInit frame, got {:?}", other),
        };

        // CRITICAL SECURITY CHECK: Verify cryptographic proof of possession of the QR secret
        let is_valid_token = verify_auth_proof(
            &self.auth_token,
            &client_ephemeral_pub,
            &client_id,
            &auth_proof,
        );

        if !is_valid_token {
            warn!("🛑 ROGUE CONNECTION ATTEMPT REJECTED! Client '{}' failed cryptographic authentication proof.", client_id);
            let resp_frame = Frame::HandshakeResponse {
                host_ephemeral_pub: [0u8; 32],
                status: 1, // Status 1 = Authentication Failed
            };
            let _ = ws_sender.send(Message::Binary(resp_frame.encode())).await;
            let _ = ws_sender.send(Message::Close(None)).await;
            bail!("Unauthorized peer connection rejected (invalid auth token proof)");
        }

        info!("🛡️ Cryptographic Auth Proof VERIFIED for device '{}'!", client_id);

        // Derive shared secret with client's public key
        let client_pub = x25519_dalek::PublicKey::from(client_ephemeral_pub);
        let shared_secret = self.host_key.secret.diffie_hellman(&client_pub);
        let crypto = SessionCrypto::from_shared_secret(shared_secret.as_bytes(), true);
        let (mut encryptor, mut decryptor) = crypto.split();

        // Record authorized device
        let mut whitelist = DeviceWhitelist::load();
        let client_pub_hex = hex::encode(client_ephemeral_pub);
        if !whitelist.authorized_keys.iter().any(|d| d.public_key_hex == client_pub_hex) {
            whitelist.authorized_keys.push(AuthorizedDevice {
                name: client_id.clone(),
                public_key_hex: client_pub_hex,
                authorized_at: chrono_now_string(),
            });
            let _ = whitelist.save();
            info!("Registered new authorized device: '{}'", client_id);
        }

        // Send HandshakeResponse (Status 0 = Success)
        let resp_frame = Frame::HandshakeResponse {
            host_ephemeral_pub: self.host_key.public_bytes(),
            status: 0,
        };
        ws_sender
            .send(Message::Binary(resp_frame.encode()))
            .await
            .context("Failed to send HandshakeResponse")?;

        info!("🔒 E2EE Session active! Connected device: '{}'", client_id);

        #[cfg(windows)]
        {
            // Channel for encrypted outbound frames
            let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
            *self.broadcaster.lock().unwrap() = Some(outbound_tx.clone());

            // Ensure default terminal (ID 0) is running
            let _ = spawn_terminal_instance(
                0,
                self.shell.clone(),
                &self.terminals,
                &self.broadcaster,
                &self.killswitch,
            );

            // Replay all active terminals and their history to client
            {
                let term_map = self.terminals.lock().unwrap();
                let mut term_ids: Vec<u32> = term_map.keys().copied().collect();
                term_ids.sort();
                for id in term_ids {
                    if let Some(term) = term_map.get(&id) {
                        if id > 0 {
                            let spawn_frame = Frame::SpawnTerminal {
                                term_id: id,
                                shell: term.shell.clone(),
                            };
                            let _ = outbound_tx.send(spawn_frame.encode());
                        }
                        let history_bytes = term.history.lock().unwrap().clone();
                        if !history_bytes.is_empty() {
                            let out_frame = Frame::TerminalOutput {
                                term_id: id,
                                data: history_bytes,
                            };
                            let _ = outbound_tx.send(out_frame.encode());
                        }
                    }
                }
            }

            // WebSocket Send Task
            let mut send_task = tokio::spawn(async move {
                while let Some(raw_frame) = outbound_rx.recv().await {
                    match encryptor.encrypt(&raw_frame) {
                        Ok(encrypted) => {
                            if ws_sender.send(Message::Binary(encrypted)).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Frame encryption error: {:?}", e);
                            break;
                        }
                    }
                }
            });

            // WebSocket Receive Task
            let terminals_recv = self.terminals.clone();
            let broadcaster_recv = self.broadcaster.clone();
            let killswitch_recv = self.killswitch.clone();
            let default_shell = self.shell.clone();
            let outbound_tx_for_ping = outbound_tx.clone();

            let mut recv_task = tokio::spawn(async move {
                while let Some(msg_res) = ws_receiver.next().await {
                    if killswitch_recv.load(Ordering::Relaxed) {
                        break;
                    }
                    match msg_res {
                        Ok(Message::Binary(ciphertext)) => {
                            match decryptor.decrypt(&ciphertext) {
                                Ok(plaintext) => match Frame::decode(&plaintext) {
                                    Ok(Frame::TerminalInput { term_id, data }) => {
                                        info!("Received TerminalInput for [{}], len={}", term_id, data.len());
                                        if let Some(term) = terminals_recv.lock().unwrap().get(&term_id) {
                                            let _ = term.write_input(&data);
                                        }
                                    }
                                    Ok(Frame::SpawnTerminal { term_id, shell }) => {
                                        let chosen_shell = if shell.is_empty() { default_shell.clone() } else { shell };
                                        let _ = spawn_terminal_instance(
                                            term_id,
                                            chosen_shell,
                                            &terminals_recv,
                                            &broadcaster_recv,
                                            &killswitch_recv,
                                        );
                                    }
                                    Ok(Frame::CloseTerminal { term_id }) => {
                                        info!("Closing ConPTY terminal [{}]", term_id);
                                        terminals_recv.lock().unwrap().remove(&term_id);
                                    }
                                    Ok(Frame::Resize { term_id, cols, rows }) => {
                                        if let Some(term) = terminals_recv.lock().unwrap().get(&term_id) {
                                            let _ = term.resize(cols, rows);
                                        }
                                    }
                                    Ok(Frame::Ping(ts)) => {
                                        let pong = Frame::Pong(ts);
                                        let _ = outbound_tx_for_ping.send(pong.encode());
                                    }
                                    Ok(Frame::Disconnect(reason)) => {
                                        info!("Client requested disconnect: {}", reason);
                                        break;
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        warn!("Frame decode error: {:?}", e);
                                    }
                                },
                                Err(e) => {
                                    error!("Frame decrypt failed (tampered / wrong key): {:?}", e);
                                    break;
                                }
                            }
                        }
                        Ok(Message::Close(_)) => {
                            info!("WebSocket closed by peer");
                            break;
                        }
                        Err(e) => {
                            warn!("WebSocket error: {:?}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            });

            tokio::select! {
                _ = &mut send_task => {},
                _ = &mut recv_task => {},
            };

            // Unbind outbound broadcaster when client disconnects, but keep persistent terminals alive!
            *self.broadcaster.lock().unwrap() = None;
            info!("Mobile client disconnected. Persistent terminals kept running for instant resumption.");

            if self.killswitch.load(Ordering::Relaxed) {
                info!("Killswitch active. Cleaning up all ConPTY processes.");
                self.terminals.lock().unwrap().clear();
            }
        }

        Ok(())
    }
}

pub fn spawn_terminal_instance(
    term_id: u32,
    shell: String,
    terminals: &SharedTerminalRegistry,
    broadcaster: &SharedBroadcaster,
    killswitch: &Arc<AtomicBool>,
) -> Result<Arc<TerminalSession>> {
    // If already exists, return existing!
    if let Some(existing) = terminals.lock().unwrap().get(&term_id) {
        return Ok(existing.clone());
    }

    info!("Spawning persistent ConPTY terminal [{}] with shell: {}", term_id, shell);
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let cmd = CommandBuilder::new(&shell);
    let _child = pair.slave.spawn_command(cmd)?;

    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;

    let session = Arc::new(TerminalSession {
        term_id,
        shell: shell.clone(),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        history: Mutex::new(Vec::new()),
        display_text: Mutex::new(String::new()),
    });

    terminals.lock().unwrap().insert(term_id, session.clone());

    let session_reader = session.clone();
    let broadcaster_clone = broadcaster.clone();
    let killswitch_clone = killswitch.clone();

    std::thread::spawn(move || {
        info!("Persistent ConPTY [{}] reader thread started", term_id);
        let mut buf = [0u8; 4096];
        while !killswitch_clone.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let chunk = &buf[..n];
                    {
                        let mut h = session_reader.history.lock().unwrap();
                        append_history(&mut h, chunk);
                    }
                    {
                        let mut d = session_reader.display_text.lock().unwrap();
                        append_display_text(&mut d, chunk);
                    }
                    if let Some(tx) = broadcaster_clone.lock().unwrap().as_ref() {
                        let frame = Frame::TerminalOutput {
                            term_id,
                            data: chunk.to_vec(),
                        };
                        let _ = tx.send(frame.encode());
                    }

                    // Auto-respond to terminal queries from PTY processes (e.g. PowerShell PSReadLine cursor probe \x1b[6n)
                    if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                        let _ = session_reader.write_input(b"\x1b[1;1R");
                    }
                    if chunk.windows(4).any(|w| w == b"\x1b[5n") {
                        let _ = session_reader.write_input(b"\x1b[0n");
                    }
                }
                Ok(_) => {
                    info!("ConPTY [{}] EOF", term_id);
                    break;
                }
                Err(e) => {
                    warn!("ConPTY [{}] read error: {:?}", term_id, e);
                    break;
                }
            }
        }
    });

    Ok(session)
}

fn append_history(history: &mut Vec<u8>, chunk: &[u8]) {
    const MAX_HISTORY: usize = 512 * 1024;
    history.extend_from_slice(chunk);
    if history.len() > MAX_HISTORY {
        let excess = history.len() - MAX_HISTORY;
        history.drain(0..excess);
    }
}

fn append_display_text(display: &mut String, chunk: &[u8]) {
    const MAX_CHARS: usize = 200_000;
    let s = String::from_utf8_lossy(chunk);
    display.push_str(&s);
    if display.len() > MAX_CHARS {
        let excess = display.len() - MAX_CHARS;
        display.drain(0..excess);
    }
}

fn chrono_now_string() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("Unix: {}", now)
}
