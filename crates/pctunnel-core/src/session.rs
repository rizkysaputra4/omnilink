use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use pctunnel_proto::{Frame, KeyPair, SessionCrypto};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

#[cfg(windows)]
use crate::conpty::win::PseudoConsole;

pub struct TunnelHostSession {
    pub relay_url: String,
    pub room_id: String,
    pub host_key: Arc<KeyPair>,
    pub pin: String,
    pub shell: String,
    pub killswitch: Arc<AtomicBool>,
}

impl TunnelHostSession {
    pub fn new(
        relay_url: String,
        room_id: String,
        host_key: Arc<KeyPair>,
        pin: String,
        shell: String,
    ) -> (Self, Arc<AtomicBool>) {
        let killswitch = Arc::new(AtomicBool::new(false));
        let session = Self {
            relay_url,
            room_id,
            host_key,
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

        info!("Connected to relay. Waiting for mobile device to scan QR code and connect...");

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // 1. Wait for counterpart connection notification from relay
        while let Some(msg_res) = ws_receiver.next().await {
            if self.killswitch.load(Ordering::Relaxed) {
                bail!("Killswitch activated before pairing");
            }
            match msg_res {
                Ok(Message::Text(t)) if t == "PCTUNNEL_PEER_CONNECTED" => {
                    info!("Mobile peer connected to room! Initiating E2EE handshake...");
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

        // 2. Perform E2EE Handshake
        // Receive HandshakeInit
        let init_msg = ws_receiver
            .next()
            .await
            .context("Stream ended before HandshakeInit")??;

        let raw_bytes = match init_msg {
            Message::Binary(b) => b,
            other => bail!("Expected binary frame for HandshakeInit, got {:?}", other),
        };

        let init_frame = Frame::decode(&raw_bytes)?;
        let (client_ephemeral_pub, client_id) = match init_frame {
            Frame::HandshakeInit { client_ephemeral_pub, client_id } => {
                (client_ephemeral_pub, client_id)
            }
            other => bail!("Expected HandshakeInit frame, got {:?}", other),
        };

        info!("Received E2EE HandshakeInit from client device: '{}'", client_id);

        // Derive shared secret with client's public key
        let client_pub = x25519_dalek::PublicKey::from(client_ephemeral_pub);
        let shared_secret = self.host_key.secret.diffie_hellman(&client_pub);
        let crypto = SessionCrypto::from_shared_secret(shared_secret.as_bytes(), true);
        let (mut encryptor, mut decryptor) = crypto.split();

        // Send HandshakeResponse
        let resp_frame = Frame::HandshakeResponse {
            host_ephemeral_pub: self.host_key.public_bytes(),
            status: 0,
        };
        ws_sender
            .send(Message::Binary(resp_frame.encode()))
            .await
            .context("Failed to send HandshakeResponse")?;

        info!("🔒 E2EE Handshake complete! Terminal session established (AES-GCM / ChaCha20-Poly1305).");

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
                    // Encrypt frame
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
                            // Decrypt frame
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
