pub mod crypto;
pub mod frames;
pub mod pairing;

pub use crypto::{KeyPair, SessionCrypto};
pub use frames::{Frame, FrameType};
pub use pairing::PairingPayload;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frames_encode_decode() {
        let frame = Frame::Resize { cols: 120, rows: 40 };
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).expect("decode resize");
        assert_eq!(frame, decoded);

        let input_frame = Frame::TerminalInput(b"ls -la\n".to_vec());
        let encoded = input_frame.encode();
        let decoded = Frame::decode(&encoded).expect("decode terminal input");
        assert_eq!(input_frame, decoded);
    }

    #[test]
    fn test_crypto_end_to_end() {
        // Generate host and client keypairs
        let host_key = KeyPair::generate();
        let client_key = KeyPair::generate();

        // Diffie-Hellman exchange
        let host_shared = host_key.secret.diffie_hellman(&client_key.public);
        let client_shared = client_key.secret.diffie_hellman(&host_key.public);

        assert_eq!(host_shared.as_bytes(), client_shared.as_bytes());

        let mut host_crypto = SessionCrypto::from_shared_secret(host_shared.as_bytes(), true);
        let mut client_crypto = SessionCrypto::from_shared_secret(client_shared.as_bytes(), false);

        // Host sends output to client
        let msg = b"Welcome to PCTunnel PowerShell Session";
        let encrypted = host_crypto.encrypt(msg).expect("encrypt host");
        let decrypted = client_crypto.decrypt(&encrypted).expect("decrypt client");
        assert_eq!(msg.to_vec(), decrypted);

        // Client sends keystroke to host
        let keystroke = b"dir\r\n";
        let encrypted = client_crypto.encrypt(keystroke).expect("encrypt client");
        let decrypted = host_crypto.decrypt(&encrypted).expect("decrypt host");
        assert_eq!(keystroke.to_vec(), decrypted);
    }
}
