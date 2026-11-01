use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use pctunnel_proto::{verify_auth_proof, Frame, KeyPair, SessionCrypto};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

#[cfg(windows)]
use crate::conpty::win::PseudoConsole;

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
        Some(PathBuf::from(home).join(".pctunnel").join("authorized_devices.json"))
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
        let killswitch = Arc::new(AtomicBool::new(false));
        let session = Self {
            relay_url,
            room_id,
            host_key,
            auth_token,
            pin,
            shell,
            killswitch: killswitch.clone(),
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
                Ok(Message::Text(t)) if t == "PCTUNNEL_PEER_CONNECTED" => {
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
            // 3. Spawn ConPTY process
            info!("Spawning shell: {}", self.shell);
            let pty = Arc::new(PseudoConsole::new(&self.shell, 80, 24)?);

            // Channel for encrypted outbound frames
            let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

            // Background task: Read from ConPTY pipe -> Encrypt -> send via outbound_tx
            let pty_read = pty.clone();
            let killswitch_pty = self.killswitch.clone();
            let outbound_tx_clone = outbound_tx.clone();

            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !killswitch_pty.load(Ordering::Relaxed) {
                    match pty_read.read_output(&mut buf) {
                        Ok(n) if n > 0 => {
                            let frame = Frame::TerminalOutput(buf[..n].to_vec());
                            if outbound_tx_clone.send(frame.encode()).is_err() {
                                break;
                            }
                        }
                        Ok(_) => break,
                        Err(e) => {
                            warn!("ConPTY read error: {:?}", e);
                            break;
                        }
                    }
                }
            });

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
            let pty_write = pty.clone();
            let killswitch_recv = self.killswitch.clone();

            let mut recv_task = tokio::spawn(async move {
                while let Some(msg_res) = ws_receiver.next().await {
                    if killswitch_recv.load(Ordering::Relaxed) {
                        break;
                    }
                    match msg_res {
                        Ok(Message::Binary(ciphertext)) => {
                            match decryptor.decrypt(&ciphertext) {
                                Ok(plaintext) => match Frame::decode(&plaintext) {
                                    Ok(Frame::TerminalInput(data)) => {
                                        let _ = pty_write.write_input(&data);
                                    }
                                    Ok(Frame::Resize { cols, rows }) => {
                                        info!("Resizing console to {}x{}", cols, rows);
                                        let _ = pty_write.resize(cols as i16, rows as i16);
                                    }
                                    Ok(Frame::Ping(ts)) => {
                                        let pong = Frame::Pong(ts);
                                        let _ = outbound_tx.send(pong.encode());
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

            info!("Session finished. Cleaning up ConPTY process.");
        }

        Ok(())
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
