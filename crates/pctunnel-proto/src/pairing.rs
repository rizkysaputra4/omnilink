use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPayload {
    /// Relay WebSocket URL, e.g. "wss://relay.findbeatradar.com/v1/tunnel"
    pub relay: String,
    /// Unique room UUID for rendezvous matching
    pub room: String,
    /// Host's X25519 public key in hex
    pub host_pub: String,
    /// 256-bit cryptographically secure one-time authorization token in hex
    pub auth_token: String,
    /// 6-digit one-time PIN for secondary visual confirmation
    pub pin: String,
}

impl PairingPayload {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json)?)
    }
}
