use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use omnilink_proto::{compute_auth_proof, Frame, KeyPair, SessionCrypto};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use uuid::Uuid;

#[tokio::test]
async fn test_live_relay_e2e_with_auth_proof() -> Result<()> {
    let relay_url = "wss://relay.findbeatradar.com/v1/tunnel";
    let room_id = format!("test-room-{}", Uuid::new_v4());
    let auth_token = [99u8; 32]; // 256-bit secret from physical QR

    let host_ws_url = format!("{}?room={}&role=host", relay_url, room_id);
    let client_ws_url = format!("{}?room={}&role=client", relay_url, room_id);

    // 1. Host connects
    let (mut host_ws, _) = connect_async(&host_ws_url).await?;
    let host_key = KeyPair::generate();

    // 2. Client connects
    let (mut client_ws, _) = connect_async(&client_ws_url).await?;
    let client_key = KeyPair::generate();

    // Both should receive PEER_CONNECTED notification from relay
    let host_msg = host_ws.next().await.unwrap()?;
    assert_eq!(host_msg, Message::Text("OMNILINK_PEER_CONNECTED".into()));

    let client_msg = client_ws.next().await.unwrap()?;
    assert_eq!(client_msg, Message::Text("OMNILINK_PEER_CONNECTED".into()));

    // 3. Client sends HandshakeInit with HMAC auth proof
    let proof = compute_auth_proof(&auth_token, &client_key.public_bytes(), "Test Device");
    let init_frame = Frame::HandshakeInit {
        client_ephemeral_pub: client_key.public_bytes(),
        client_id: "Test Device".into(),
        auth_proof: proof,
    };
    client_ws.send(Message::Binary(init_frame.encode())).await?;

    // 4. Host receives HandshakeInit and validates proof
    let raw_init = host_ws.next().await.unwrap()?;
    let raw_bytes = match raw_init {
        Message::Binary(b) => b,
        other => panic!("Expected binary, got {:?}", other),
    };
    let decoded_init = Frame::decode(&raw_bytes)?;
    match decoded_init {
        Frame::HandshakeInit { client_ephemeral_pub, client_id, auth_proof: received_proof } => {
            assert!(omnilink_proto::verify_auth_proof(&auth_token, &client_ephemeral_pub, &client_id, &received_proof));
        }
        other => panic!("Unexpected frame: {:?}", other),
    }

    // 5. Host sends HandshakeResponse (Status 0 = OK)
    let resp_frame = Frame::HandshakeResponse {
        host_ephemeral_pub: host_key.public_bytes(),
        status: 0,
    };
    host_ws.send(Message::Binary(resp_frame.encode())).await?;

    // 6. Client receives HandshakeResponse
    let raw_resp = client_ws.next().await.unwrap()?;
    let resp_bytes = match raw_resp {
        Message::Binary(b) => b,
        other => panic!("Expected binary, got {:?}", other),
    };
    let decoded_resp = Frame::decode(&resp_bytes)?;
    match decoded_resp {
        Frame::HandshakeResponse { status, .. } => assert_eq!(status, 0),
        other => panic!("Unexpected frame: {:?}", other),
    }

    // 7. Establish symmetric ChaCha20-Poly1305 sessions
    let shared_host = host_key.secret.diffie_hellman(&client_key.public);
    let shared_client = client_key.secret.diffie_hellman(&host_key.public);
    assert_eq!(shared_host.as_bytes(), shared_client.as_bytes());

    let mut host_crypto = SessionCrypto::from_shared_secret(shared_host.as_bytes(), true);
    let mut client_crypto = SessionCrypto::from_shared_secret(shared_client.as_bytes(), false);

    // 8. Test Encrypted Keystroke from Client -> Host
    let input_frame = Frame::TerminalInput(b"whoami\r\n".to_vec());
    let encrypted_input = client_crypto.encrypt(&input_frame.encode())?;
    client_ws.send(Message::Binary(encrypted_input)).await?;

    let host_received_enc = match host_ws.next().await.unwrap()? {
        Message::Binary(b) => b,
        other => panic!("Expected binary, got {:?}", other),
    };
    let host_decrypted = host_crypto.decrypt(&host_received_enc)?;
    let host_frame = Frame::decode(&host_decrypted)?;
    assert_eq!(host_frame, Frame::TerminalInput(b"whoami\r\n".to_vec()));

    // 9. Test Encrypted Output from Host -> Client
    let output_frame = Frame::TerminalOutput(b"root\r\n".to_vec());
    let encrypted_output = host_crypto.encrypt(&output_frame.encode())?;
    host_ws.send(Message::Binary(encrypted_output)).await?;

    let client_received_enc = match client_ws.next().await.unwrap()? {
        Message::Binary(b) => b,
        other => panic!("Expected binary, got {:?}", other),
    };
    let client_decrypted = client_crypto.decrypt(&client_received_enc)?;
    let client_frame = Frame::decode(&client_decrypted)?;
    assert_eq!(client_frame, Frame::TerminalOutput(b"root\r\n".to_vec()));

    println!("✅ Live E2EE Relay Test on wss://relay.findbeatradar.com passed with 100% success!");
    Ok(())
}
