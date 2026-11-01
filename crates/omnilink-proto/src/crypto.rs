use anyhow::{bail, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct KeyPair {
    pub secret: StaticSecret,
    pub public: PublicKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        *self.public.as_bytes()
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.public_bytes())
    }

    pub fn from_secret_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }
}

pub struct SessionEncryptor {
    cipher_send: ChaCha20Poly1305,
    send_nonce: u64,
}

impl SessionEncryptor {
    fn make_nonce(counter: u64) -> Nonce {
        let mut n = [0u8; 12];
        n[4..12].copy_from_slice(&counter.to_be_bytes());
        *Nonce::from_slice(&n)
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::make_nonce(self.send_nonce);
        let ciphertext = self
            .cipher_send
            .encrypt(&nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption error: {:?}", e))?;

        let mut out = Vec::with_capacity(8 + ciphertext.len());
        out.extend_from_slice(&self.send_nonce.to_be_bytes());
        out.extend_from_slice(&ciphertext);
        self.send_nonce += 1;
        Ok(out)
    }
}

pub struct SessionDecryptor {
    cipher_recv: ChaCha20Poly1305,
    recv_nonce: u64,
}

impl SessionDecryptor {
    fn make_nonce(counter: u64) -> Nonce {
        let mut n = [0u8; 12];
        n[4..12].copy_from_slice(&counter.to_be_bytes());
        *Nonce::from_slice(&n)
    }

    pub fn decrypt(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 8 {
            bail!("Ciphertext too short (< 8 bytes nonce)");
        }
        let nonce_val = u64::from_be_bytes(data[0..8].try_into()?);
        if nonce_val < self.recv_nonce {
            bail!("Replay attack detected: nonce {} < expected {}", nonce_val, self.recv_nonce);
        }
        self.recv_nonce = nonce_val + 1;

        let nonce = Self::make_nonce(nonce_val);
        let plaintext = self
            .cipher_recv
            .decrypt(&nonce, &data[8..])
            .map_err(|e| anyhow::anyhow!("Decryption / integrity check failed: {:?}", e))?;
        Ok(plaintext)
    }
}

pub struct SessionCrypto {
    pub send: SessionEncryptor,
    pub recv: SessionDecryptor,
}

impl SessionCrypto {
    pub fn from_shared_secret(shared_secret: &[u8; 32], is_host: bool) -> Self {
        let mut hasher_hc = Sha256::new();
        hasher_hc.update(shared_secret);
        hasher_hc.update(b"OMNILINK_HOST_TO_CLIENT_V1");
        let key_hc = hasher_hc.finalize();

        let mut hasher_ch = Sha256::new();
        hasher_ch.update(shared_secret);
        hasher_ch.update(b"OMNILINK_CLIENT_TO_HOST_V1");
        let key_ch = hasher_ch.finalize();

        let (send_key, recv_key) = if is_host {
            (key_hc, key_ch)
        } else {
            (key_ch, key_hc)
        };

        let cipher_send = ChaCha20Poly1305::new_from_slice(&send_key).expect("32 bytes key");
        let cipher_recv = ChaCha20Poly1305::new_from_slice(&recv_key).expect("32 bytes key");

        Self {
            send: SessionEncryptor {
                cipher_send,
                send_nonce: 0,
            },
            recv: SessionDecryptor {
                cipher_recv,
                recv_nonce: 0,
            },
        }
    }

    pub fn split(self) -> (SessionEncryptor, SessionDecryptor) {
        (self.send, self.recv)
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.send.encrypt(plaintext)
    }

    pub fn decrypt(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.recv.decrypt(data)
    }
}
