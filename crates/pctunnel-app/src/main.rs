use std::sync::{
    atomic::Ordering,
    Arc,
};

use clap::Parser;
use pctunnel_core::TunnelHostSession;
use pctunnel_proto::{KeyPair, PairingPayload};
use qrcode::{render::unicode, QrCode};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "pctunnel-app",
    about = "PCTunnel Windows Host Agent with ConPTY & Zero-Trust E2EE"
)]
struct Args {
    /// Relay WebSocket URL
    #[arg(
        short,
        long,
        default_value = "wss://relay.findbeatradar.com",
        env = "PCTUNNEL_RELAY"
    )]
    relay: String,

    /// Shell executable to launch (e.g. powershell.exe, cmd.exe, bash.exe)
    #[arg(short, long, env = "PCTUNNEL_SHELL")]
    shell: Option<String>,

    /// Custom Room UUID (leave empty to generate randomly)
    #[arg(long)]
    room: Option<String>,
}

fn detect_best_shell() -> String {
    // Priority: pwsh.exe (PowerShell 7+) -> powershell.exe -> cmd.exe
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pctunnel_app=info,pctunnel_core=info".into()),
        )
        .init();

    let args = Args::parse();
    let shell = args.shell.unwrap_or_else(detect_best_shell);
    let room_id = args.room.unwrap_or_else(|| Uuid::new_v4().to_string());
    let pin = format!("{:06}", rand::random::<u32>() % 1_000_000);

    let host_key = Arc::new(KeyPair::generate());
    let host_pub_hex = hex::encode(host_key.public_bytes());

    let pairing = PairingPayload {
        relay: args.relay.clone(),
        room: room_id.clone(),
        host_pub: host_pub_hex.clone(),
        pin: pin.clone(),
    };

    let pairing_json = pairing.to_json()?;

    // Generate terminal QR Code
    let qr = QrCode::new(pairing_json.as_bytes())?;
    let qr_string = qr
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();

    println!("\n==================================================================");
    println!("             🔒 PCTunnel Windows Host Agent v0.1.0                ");
    println!("         Zero-Trust Encrypted Remote Terminal Bridge              ");
    println!("==================================================================\n");

    println!("Scan this QR Code using the PCTunnel Android App:\n");
    println!("{}", qr_string);
    println!("\n------------------------------------------------------------------");
    println!("  Relay Server : {}", args.relay);
    println!("  Room UUID    : {}", room_id);
    println!("  One-Time PIN : {}", pin);
    println!("  Target Shell : {}", shell);
    println!("  Security     : X25519 + ChaCha20-Poly1305 (E2EE)");
    println!("------------------------------------------------------------------");
    println!("  Type 'q' + Enter or press Ctrl+C at any time for [KILLSWITCH]   ");
    println!("==================================================================\n");

    let (session, killswitch) = TunnelHostSession::new(
        args.relay,
        room_id,
        host_key,
        pin,
        shell,
    );

    // Spawn Killswitch console watcher
    let ks_watcher = killswitch.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line).is_ok() {
            if line.trim().eq_ignore_ascii_case("q") || line.trim().eq_ignore_ascii_case("stop") {
                println!("\n🛑 [KILLSWITCH ACTIVATED BY USER] Terminating terminal session immediately!");
                ks_watcher.store(true, Ordering::SeqCst);
                break;
            }
            line.clear();
        }
    });

    // Run session
    if let Err(e) = session.run().await {
        warn!("Session ended with: {:?}", e);
    }

    info!("PCTunnel host agent finished.");
    Ok(())
}
