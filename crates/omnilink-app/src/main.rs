use std::sync::{
    atomic::Ordering,
    Arc,
};

use clap::Parser;
use omnilink_core::TunnelHostSession;
use omnilink_proto::{KeyPair, PairingPayload};
use qrcode::{render::unicode, QrCode};
use rand::RngCore;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "omnilink-app",
    about = "OmniLink Windows Host - Modern Minimalist Zero-Trust Terminal Bridge"
)]
struct Args {
    /// Relay WebSocket URL
    #[arg(
        short,
        long,
        default_value = "wss://relay.findbeatradar.com",
        env = "OMNILINK_RELAY"
    )]
    relay: String,

    /// Shell executable to launch (e.g. powershell.exe, pwsh.exe, cmd.exe, bash.exe)
    #[arg(short, long, env = "OMNILINK_SHELL")]
    shell: Option<String>,

    /// Custom Room UUID (leave empty to generate randomly)
    #[arg(long)]
    room: Option<String>,
}

fn detect_best_shell() -> String {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let default_powershell = format!("{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe", system_root);

    if let Ok(path) = which("pwsh.exe") {
        return path;
    }
    if std::path::Path::new(&default_powershell).exists() {
        return default_powershell;
    }
    "cmd.exe".to_string()
}

fn which(exe_name: &str) -> Result<String, ()> {
    if let Ok(paths) = std::env::var("PATH") {
        for p in std::env::split_paths(&paths) {
            let full_path = p.join(exe_name);
            if full_path.is_file() {
                return Ok(full_path.to_string_lossy().to_string());
            }
        }
    }
    Err(())
}

fn get_local_ip() -> String {
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".into()
}

fn get_current_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omnilink_app=info,omnilink_core=info".into()),
        )
        .init();

    let args = Args::parse();
    let shell = args.shell.unwrap_or_else(detect_best_shell);
    let room_id = args.room.unwrap_or_else(|| Uuid::new_v4().to_string());
    let pin = format!("{:06}", rand::random::<u32>() % 1_000_000);

    // Cryptographic authorization secret: 256 bits of high entropy
    let mut auth_token = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut auth_token);
    let auth_token_hex = hex::encode(auth_token);

    let host_key = Arc::new(KeyPair::generate());
    let host_pub_hex = hex::encode(host_key.public_bytes());

    let pairing = PairingPayload {
        relay: args.relay.clone(),
        room: room_id.clone(),
        host_pub: host_pub_hex.clone(),
        auth_token: auth_token_hex.clone(),
        pin: pin.clone(),
    };

    let pairing_url = pairing.to_url()?;
    let pairing_json = pairing.to_json()?;

    // Generate terminal QR Code using the public HTTPS pairing URL
    let qr = QrCode::new(pairing_url.as_bytes())?;
    let qr_string = qr
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();

    let _local_ip = get_local_ip();

    // Modern Minimalist Dark UI
    print!("\x1B[2J\x1B[1;1H"); // Clear screen
    println!("\x1b[38;2;16;185;129m╭─────────────────────────────────────────────────────────────────────╮\x1b[0m");
    println!("\x1b[38;2;16;185;129m│\x1b[0m   \x1b[1;38;2;255;255;255mOMNILINK\x1b[0m  \x1b[38;2;100;116;139m│\x1b[0m  \x1b[38;2;16;185;129m● HOST ACTIVE\x1b[0m  \x1b[38;2;100;116;139m(Zero-Trust E2EE ConPTY Bridge)\x1b[0m         \x1b[38;2;16;185;129m│\x1b[0m");
    println!("\x1b[38;2;16;185;129m╰─────────────────────────────────────────────────────────────────────╯\x1b[0m\n");

    println!("  \x1b[1;38;2;0;242;254m[ 1. SCAN TO CONNECT (Zero-Config / Remote) ]\x1b[0m");
    println!("  Scan this QR code from the OmniLink Android App or Phone Camera:\n");

    for line in qr_string.lines() {
        println!("    {}", line);
    }

    println!("\n  \x1b[38;2;148;163;184m╭─ Connection Details ───────────────────────────────────────────────╮\x1b[0m");
    println!("  \x1b[38;2;148;163;184m│\x1b[0m  \x1b[38;2;100;116;139mPairing URL  :\x1b[0m \x1b[38;2;56;189;248m{}\x1b[0m", pairing_url);
    println!("  \x1b[38;2;148;163;184m│\x1b[0m  \x1b[38;2;100;116;139mRelay Host   :\x1b[0m \x1b[1;38;2;241;245;249m{}\x1b[0m", args.relay);
    println!("  \x1b[38;2;148;163;184m│\x1b[0m  \x1b[38;2;100;116;139mRoom UUID    :\x1b[0m \x1b[38;2;56;189;248m{}\x1b[0m", room_id);
    println!("  \x1b[38;2;148;163;184m│\x1b[0m  \x1b[38;2;100;116;139mAuth Token   :\x1b[0m \x1b[38;2;168;85;247m{}...\x1b[0m (256-bit cryptosecret)", &auth_token_hex[..12]);
    println!("  \x1b[38;2;148;163;184m│\x1b[0m  \x1b[38;2;100;116;139mTarget Shell :\x1b[0m \x1b[38;2;241;245;249m{}\x1b[0m", shell);
    println!("  \x1b[38;2;148;163;184m│\x1b[0m  \x1b[38;2;100;116;139mSecurity     :\x1b[0m \x1b[38;2;16;185;129mChaCha20-Poly1305 + X25519 (Proof-Gated)\x1b[0m");
    println!("  \x1b[38;2;148;163;184m╰────────────────────────────────────────────────────────────────────╯\x1b[0m");
    println!("  \x1b[38;2;100;116;139mOr copy raw JSON payload:\x1b[0m {}", pairing_json);

    println!("\n  \x1b[38;2;239;68;68m[KILLSWITCH]\x1b[0m Type \x1b[1;38;2;255;255;255m'q'\x1b[0m or press \x1b[1;38;2;255;255;255mCtrl+C\x1b[0m to instantly terminate session.");
    println!("  Waiting for authorized peer handshake...\n");

    let (session, killswitch) = TunnelHostSession::new(
        args.relay,
        room_id,
        host_key,
        auth_token,
        pin,
        shell,
    );

    // Spawn Killswitch console watcher
    let ks_watcher = killswitch.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line).is_ok() {
            if line.trim().eq_ignore_ascii_case("q") || line.trim().eq_ignore_ascii_case("stop") {
                println!("\n\x1b[1;38;2;239;68;68m🛑 [KILLSWITCH ACTIVATED] All processes wiped. Tunnel severed.\x1b[0m");
                ks_watcher.store(true, Ordering::SeqCst);
                break;
            }
            line.clear();
        }
    });

    if let Err(e) = session.run().await {
        warn!("Session ended with: {:?}", e);
    }

    info!("OmniLink host agent finished.");
    Ok(())
}
