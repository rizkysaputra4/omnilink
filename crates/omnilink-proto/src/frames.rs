use anyhow::{bail, Result};
use bytes::{Buf, BufMut, BytesMut};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub fn compute_auth_proof(
    auth_token: &[u8; 32],
    client_ephemeral_pub: &[u8; 32],
    client_id: &str,
) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(auth_token).expect("HMAC can take key of any size");
    mac.update(client_ephemeral_pub);
    mac.update(client_id.as_bytes());
    let result = mac.finalize();
    let bytes = result.into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

pub fn verify_auth_proof(
    auth_token: &[u8; 32],
    client_ephemeral_pub: &[u8; 32],
    client_id: &str,
    expected_proof: &[u8; 32],
) -> bool {
    let computed = compute_auth_proof(auth_token, client_ephemeral_pub, client_id);
    computed.ct_eq(expected_proof).into()
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    HandshakeInit = 0x01,
    HandshakeResponse = 0x02,
    TerminalInput = 0x03,
    TerminalOutput = 0x04,
    Resize = 0x05,
    Ping = 0x06,
    Pong = 0x07,
    Disconnect = 0x08,
    SpawnTerminal = 0x09,
    CloseTerminal = 0x0A,
}

impl TryFrom<u8> for FrameType {
    type Error = anyhow::Error;

    fn try_from(v: u8) -> Result<Self> {
        match v {
            0x01 => Ok(FrameType::HandshakeInit),
            0x02 => Ok(FrameType::HandshakeResponse),
            0x03 => Ok(FrameType::TerminalInput),
            0x04 => Ok(FrameType::TerminalOutput),
            0x05 => Ok(FrameType::Resize),
            0x06 => Ok(FrameType::Ping),
            0x07 => Ok(FrameType::Pong),
            0x08 => Ok(FrameType::Disconnect),
            0x09 => Ok(FrameType::SpawnTerminal),
            0x0A => Ok(FrameType::CloseTerminal),
            other => bail!("Unknown frame type: 0x{:02X}", other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    HandshakeInit {
        client_ephemeral_pub: [u8; 32],
        client_id: String,
        auth_proof: [u8; 32],
    },
    HandshakeResponse {
        host_ephemeral_pub: [u8; 32],
        status: u8, // 0 = success, 1 = auth failed, 2 = rejected
    },
    TerminalInput {
        term_id: u32,
        data: Vec<u8>,
    },
    TerminalOutput {
        term_id: u32,
        data: Vec<u8>,
    },
    Resize {
        term_id: u32,
        cols: u16,
        rows: u16,
    },
    SpawnTerminal {
        term_id: u32,
        shell: String,
    },
    CloseTerminal {
        term_id: u32,
    },
    Ping(u64),
    Pong(u64),
    Disconnect(String),
}

impl Frame {
    pub fn frame_type(&self) -> FrameType {
        match self {
            Frame::HandshakeInit { .. } => FrameType::HandshakeInit,
            Frame::HandshakeResponse { .. } => FrameType::HandshakeResponse,
            Frame::TerminalInput { .. } => FrameType::TerminalInput,
            Frame::TerminalOutput { .. } => FrameType::TerminalOutput,
            Frame::Resize { .. } => FrameType::Resize,
            Frame::SpawnTerminal { .. } => FrameType::SpawnTerminal,
            Frame::CloseTerminal { .. } => FrameType::CloseTerminal,
            Frame::Ping(_) => FrameType::Ping,
            Frame::Pong(_) => FrameType::Pong,
            Frame::Disconnect(_) => FrameType::Disconnect,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let ftype = self.frame_type() as u8;
        let mut payload = BytesMut::new();

        match self {
            Frame::HandshakeInit {
                client_ephemeral_pub,
                client_id,
                auth_proof,
            } => {
                payload.put_slice(client_ephemeral_pub);
                payload.put_slice(auth_proof);
                let id_bytes = client_id.as_bytes();
                payload.put_u16(id_bytes.len() as u16);
                payload.put_slice(id_bytes);
            }
            Frame::HandshakeResponse { host_ephemeral_pub, status } => {
                payload.put_slice(host_ephemeral_pub);
                payload.put_u8(*status);
            }
            Frame::TerminalInput { term_id, data } => {
                payload.put_u32(*term_id);
                payload.put_slice(data);
            }
            Frame::TerminalOutput { term_id, data } => {
                payload.put_u32(*term_id);
                payload.put_slice(data);
            }
            Frame::Resize { term_id, cols, rows } => {
                payload.put_u32(*term_id);
                payload.put_u16(*cols);
                payload.put_u16(*rows);
            }
            Frame::SpawnTerminal { term_id, shell } => {
                payload.put_u32(*term_id);
                let s_bytes = shell.as_bytes();
                payload.put_u16(s_bytes.len() as u16);
                payload.put_slice(s_bytes);
            }
            Frame::CloseTerminal { term_id } => {
                payload.put_u32(*term_id);
            }
            Frame::Ping(ts) => {
                payload.put_u64(*ts);
            }
            Frame::Pong(ts) => {
                payload.put_u64(*ts);
            }
            Frame::Disconnect(reason) => {
                payload.put_slice(reason.as_bytes());
            }
        }

        let mut buf = BytesMut::with_capacity(1 + 4 + payload.len());
        buf.put_u8(ftype);
        buf.put_u32(payload.len() as u32);
        buf.put_slice(&payload);
        buf.to_vec()
    }

    pub fn decode(mut raw: &[u8]) -> Result<Self> {
        if raw.len() < 5 {
            bail!("Frame too short (< 5 bytes)");
        }
        let ftype = FrameType::try_from(raw.get_u8())?;
        let len = raw.get_u32() as usize;
        if raw.remaining() < len {
            bail!("Incomplete frame payload: expected {} bytes, got {}", len, raw.remaining());
        }
        let mut payload = &raw[..len];

        match ftype {
            FrameType::HandshakeInit => {
                if payload.len() < 66 {
                    bail!("HandshakeInit payload too short (< 66 bytes)");
                }
                let mut client_ephemeral_pub = [0u8; 32];
                payload.copy_to_slice(&mut client_ephemeral_pub);
                let mut auth_proof = [0u8; 32];
                payload.copy_to_slice(&mut auth_proof);

                let id_len = payload.get_u16() as usize;
                if payload.remaining() < id_len {
                    bail!("HandshakeInit invalid client_id length");
                }
                let client_id = String::from_utf8_lossy(&payload[..id_len]).to_string();
                Ok(Frame::HandshakeInit {
                    client_ephemeral_pub,
                    client_id,
                    auth_proof,
                })
            }
            FrameType::HandshakeResponse => {
                if payload.len() < 33 {
                    bail!("HandshakeResponse payload too short");
                }
                let mut host_ephemeral_pub = [0u8; 32];
                payload.copy_to_slice(&mut host_ephemeral_pub);
                let status = payload.get_u8();
                Ok(Frame::HandshakeResponse { host_ephemeral_pub, status })
            }
            FrameType::TerminalInput => {
                if payload.len() >= 4 {
                    let term_id = payload.get_u32();
                    let data = payload.to_vec();
                    Ok(Frame::TerminalInput { term_id, data })
                } else {
                    Ok(Frame::TerminalInput { term_id: 0, data: payload.to_vec() })
                }
            }
            FrameType::TerminalOutput => {
                if payload.len() >= 4 {
                    let term_id = payload.get_u32();
                    let data = payload.to_vec();
                    Ok(Frame::TerminalOutput { term_id, data })
                } else {
                    Ok(Frame::TerminalOutput { term_id: 0, data: payload.to_vec() })
                }
            }
            FrameType::Resize => {
                if payload.len() >= 8 {
                    let term_id = payload.get_u32();
                    let cols = payload.get_u16();
                    let rows = payload.get_u16();
                    Ok(Frame::Resize { term_id, cols, rows })
                } else if payload.len() >= 4 {
                    let cols = payload.get_u16();
                    let rows = payload.get_u16();
                    Ok(Frame::Resize { term_id: 0, cols, rows })
                } else {
                    bail!("Resize payload too short");
                }
            }
            FrameType::SpawnTerminal => {
                if payload.len() < 6 {
                    bail!("SpawnTerminal payload too short");
                }
                let term_id = payload.get_u32();
                let s_len = payload.get_u16() as usize;
                let shell = if payload.remaining() >= s_len {
                    String::from_utf8_lossy(&payload[..s_len]).to_string()
                } else {
                    String::new()
                };
                Ok(Frame::SpawnTerminal { term_id, shell })
            }
            FrameType::CloseTerminal => {
                if payload.len() < 4 {
                    bail!("CloseTerminal payload too short");
                }
                let term_id = payload.get_u32();
                Ok(Frame::CloseTerminal { term_id })
            }
            FrameType::Ping => {
                if payload.len() < 8 {
                    bail!("Ping payload too short");
                }
                let ts = payload.get_u64();
                Ok(Frame::Ping(ts))
            }
            FrameType::Pong => {
                if payload.len() < 8 {
                    bail!("Pong payload too short");
                }
                let ts = payload.get_u64();
                Ok(Frame::Pong(ts))
            }
            FrameType::Disconnect => {
                let reason = String::from_utf8_lossy(payload).to_string();
                Ok(Frame::Disconnect(reason))
            }
        }
    }
}
