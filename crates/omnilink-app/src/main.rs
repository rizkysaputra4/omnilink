#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use clap::Parser;
use eframe::egui::{self, Color32, RichText, Vec2};
use omnilink_core::TunnelHostSession;
use omnilink_proto::{KeyPair, PairingPayload};
use qrcode::QrCode;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "omnilink-app",
    about = "OmniLink Windows Host - Modern Minimalist Zero-Trust Terminal Bridge"
)]
struct Args {
    /// Run in headless / CLI mode without GUI
    #[arg(long)]
    cli: bool,

    /// Relay WebSocket URL
    #[arg(
        short,
        long,
        default_value = "wss://relay.findbeatradar.com",
        env = "OMNILINK_RELAY"
    )]
    relay: String,

    /// Shell executable to launch (e.g. powershell.exe, cmd.exe)
    #[arg(short, long, env = "OMNILINK_SHELL")]
    shell: Option<String>,
}

// ==============================================================================
// Persistent Host Configuration (~/.omnilink/host_config.json)
// Ensures the PC retains its identity so mobile never needs to re-scan QR!
// ==============================================================================

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HostConfig {
    pub room_id: String,
    pub auth_token_hex: String,
    pub host_secret_hex: String,
    pub pin: String,
    pub relay: String,
    pub shell: String,
}

impl HostConfig {
    pub fn config_path() -> PathBuf {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".omnilink").join("host_config.json")
    }

    pub fn load_or_create(default_relay: &str, default_shell: &str) -> Self {
        let path = Self::config_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<Self>(&content) {
                return config;
            }
        }

        // Generate brand new persistent credentials
        let config = Self::generate_new(default_relay, default_shell);
        config.save();
        config
    }

    pub fn generate_new(relay: &str, shell: &str) -> Self {
        let room_id = Uuid::new_v4().to_string();
        let mut token = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut token);
        let keypair = KeyPair::generate();
        let pin = format!("{:06}", (rand::random::<u32>() % 900_000) + 100_000);

        Self {
            room_id,
            auth_token_hex: hex::encode(token),
            host_secret_hex: hex::encode(keypair.secret.to_bytes()),
            pin,
            relay: relay.to_string(),
            shell: shell.to_string(),
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn get_keypair(&self) -> KeyPair {
        if let Ok(bytes) = hex::decode(&self.host_secret_hex) {
            if bytes.len() == 32 {
                let mut secret_bytes = [0u8; 32];
                secret_bytes.copy_from_slice(&bytes);
                return KeyPair::from_secret_bytes(secret_bytes);
            }
        }
        KeyPair::generate()
    }

    pub fn get_auth_token(&self) -> [u8; 32] {
        let mut token = [0u8; 32];
        if let Ok(bytes) = hex::decode(&self.auth_token_hex) {
            if bytes.len() == 32 {
                token.copy_from_slice(&bytes);
            }
        }
        token
    }

    pub fn get_pairing_payload(&self) -> PairingPayload {
        let keypair = self.get_keypair();
        PairingPayload {
            relay: self.relay.clone(),
            room: self.room_id.clone(),
            host_pub: keypair.public_hex(),
            auth_token: self.auth_token_hex.clone(),
            pin: self.pin.clone(),
        }
    }
}

fn detect_best_shell() -> String {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let default_powershell = format!(
        "{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        system_root
    );
    if std::path::Path::new(&default_powershell).exists() {
        return default_powershell;
    }
    "cmd.exe".to_string()
}

// ==============================================================================
// Main Entrypoint (GUI vs CLI)
// ==============================================================================

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omnilink_app=info,omnilink_core=info".into()),
        )
        .init();

    let args = Args::parse();
    let default_shell = args.shell.unwrap_or_else(detect_best_shell);
    let config = HostConfig::load_or_create(&args.relay, &default_shell);

    if args.cli {
        // Run Headless / Console Mode
        return run_cli_mode(config);
    }

    // Launch Modern Native Windows GUI
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 710.0])
            .with_min_inner_size([420.0, 640.0])
            .with_title("OmniLink - Secure Zero-Trust Terminal Bridge")
            .with_active(true),
        ..Default::default()
    };

    eframe::run_native(
        "OmniLink Desktop",
        options,
        Box::new(|cc| Ok(Box::new(OmniLinkGui::new(cc, config)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI Error: {}", e))
}

// ==============================================================================
// GUI Application State
// ==============================================================================

struct OmniLinkGui {
    config: HostConfig,
    is_running: bool,
    status_text: String,
    active_killswitch: Option<Arc<AtomicBool>>,
    qr_texture: Option<egui::TextureHandle>,
    pairing_url: String,
    pairing_json: String,
    logs: Arc<Mutex<Vec<String>>>,
    selected_shell: String,
}

impl OmniLinkGui {
    fn new(cc: &eframe::CreationContext<'_>, config: HostConfig) -> Self {
        let mut visual = egui::Visuals::dark();
        visual.override_text_color = Some(Color32::from_rgb(241, 245, 249));
        visual.panel_fill = Color32::from_rgb(9, 10, 15);
        cc.egui_ctx.set_visuals(visual);

        let pairing = config.get_pairing_payload();
        let pairing_url = pairing.to_url().unwrap_or_default();
        let pairing_json = pairing.to_json().unwrap_or_default();

        let qr_texture = Self::generate_qr_texture(&cc.egui_ctx, &pairing_url);

        let mut app = Self {
            selected_shell: config.shell.clone(),
            config,
            is_running: false,
            status_text: "Service Stopped (Ready to start)".into(),
            active_killswitch: None,
            qr_texture,
            pairing_url,
            pairing_json,
            logs: Arc::new(Mutex::new(vec![
                "🔒 OmniLink Zero-Trust Host Initialized".into(),
                "Permanent pairing identity loaded from ~/.omnilink/host_config.json".into(),
            ])),
        };

        // Automatically start the session by default for instant connectivity
        app.start_service();
        app
    }

    fn generate_qr_texture(ctx: &egui::Context, url: &str) -> Option<egui::TextureHandle> {
        let qr = QrCode::new(url.as_bytes()).ok()?;
        let width = qr.width();
        let border = 2;
        let total_w = width + border * 2;
        let mut pixels = Vec::with_capacity(total_w * total_w);

        for y in 0..total_w {
            for x in 0..total_w {
                let is_dark = if x >= border && x < width + border && y >= border && y < width + border {
                    qr[(x - border, y - border)] == qrcode::Color::Dark
                } else {
                    false
                };

                pixels.push(if is_dark {
                    Color32::from_rgb(0, 0, 0)
                } else {
                    Color32::from_rgb(255, 255, 255)
                });
            }
        }

        let image = egui::ColorImage {
            size: [total_w, total_w],
            pixels,
        };

        Some(ctx.load_texture("omnilink_qr", image, egui::TextureOptions::NEAREST))
    }

    fn start_service(&mut self) {
        if self.is_running {
            return;
        }

        let keypair = Arc::new(self.config.get_keypair());
        let auth_token = self.config.get_auth_token();
        let pin = self.config.pin.clone();
        let room_id = self.config.room_id.clone();
        let relay_url = self.config.relay.clone();
        let shell = self.selected_shell.clone();

        let (session, killswitch) = TunnelHostSession::new(
            relay_url,
            room_id,
            keypair,
            auth_token,
            pin,
            shell,
        );

        self.active_killswitch = Some(killswitch.clone());
        self.is_running = true;
        self.status_text = "🟢 Active - Listening for Paired Phone...".into();

        let logs = self.logs.clone();
        logs.lock().unwrap().push("🚀 Tunnel Service Started. Connecting to Blind Relay...".into());

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            if let Ok(rt) = rt {
                rt.block_on(async {
                    if let Err(e) = session.run().await {
                        if !killswitch.load(Ordering::SeqCst) {
                            warn!("Session error: {}", e);
                            logs.lock().unwrap().push(format!("⚠️ Session interrupted: {}", e));
                        }
                    }
                });
            }
        });
    }

    fn stop_service(&mut self) {
        if let Some(ks) = self.active_killswitch.take() {
            ks.store(true, Ordering::SeqCst);
        }
        self.is_running = false;
        self.status_text = "🔴 Service Stopped".into();
        self.logs.lock().unwrap().push("⏹ Service stopped by user".into());
    }

    fn regenerate_pairing(&mut self, ctx: &egui::Context) {
        self.stop_service();
        self.config = HostConfig::generate_new(&self.config.relay, &self.selected_shell);
        self.config.save();

        let pairing = self.config.get_pairing_payload();
        self.pairing_url = pairing.to_url().unwrap_or_default();
        self.pairing_json = pairing.to_json().unwrap_or_default();
        self.qr_texture = Self::generate_qr_texture(ctx, &self.pairing_url);

        self.logs.lock().unwrap().push("🔑 Generated new persistent Room ID & Security Keys".into());
        self.start_service();
    }
}

impl eframe::App for OmniLinkGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);

            // Header Banner
            ui.horizontal(|ui| {
                ui.label(RichText::new("OMNILINK").size(20.0).strong().color(Color32::WHITE));
                ui.label(RichText::new("HOST").size(11.0).strong().background_color(Color32::from_rgb(30, 41, 59)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.is_running {
                        ui.label(RichText::new("● ONLINE").size(12.0).strong().color(Color32::from_rgb(16, 185, 129)));
                    } else {
                        ui.label(RichText::new("● STOPPED").size(12.0).strong().color(Color32::from_rgb(239, 68, 68)));
                    }
                });
            });

            ui.add_space(10.0);

            // Primary START / STOP Big Action Button
            let btn_size = Vec2::new(ui.available_width(), 48.0);
            if self.is_running {
                let stop_btn = egui::Button::new(
                    RichText::new("⏹  STOP SERVICE")
                        .size(15.0)
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(Color32::from_rgb(220, 38, 38))
                .min_size(btn_size);

                if ui.add(stop_btn).clicked() {
                    self.stop_service();
                }
            } else {
                let start_btn = egui::Button::new(
                    RichText::new("▶  START SERVICE")
                        .size(15.0)
                        .strong()
                        .color(Color32::BLACK),
                )
                .fill(Color32::from_rgb(0, 242, 254))
                .min_size(btn_size);

                if ui.add(start_btn).clicked() {
                    self.start_service();
                }
            }

            ui.add_space(12.0);

            // QR Code Box (Zero-Trust Physical Scan)
            egui::Frame::none()
                .fill(Color32::from_rgb(18, 20, 29))
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 43, 58)))
                .rounding(14.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("SCAN TO PAIR PHONE").size(14.0).strong().color(Color32::WHITE));
                        ui.label(
                            RichText::new("Scan once from OmniLink Android App to bind permanently")
                                .size(11.0)
                                .color(Color32::from_rgb(148, 163, 184)),
                        );

                        ui.add_space(8.0);

                        if let Some(texture) = &self.qr_texture {
                            ui.image((texture.id(), Vec2::new(170.0, 170.0)));
                        }

                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            if ui.button("📋 Copy 1-Click Link").clicked() {
                                ui.output_mut(|o| o.copied_text = self.pairing_url.clone());
                                self.logs.lock().unwrap().push("📋 Copied pairing link to clipboard".into());
                            }
                            if ui.button("📋 Copy JSON").clicked() {
                                ui.output_mut(|o| o.copied_text = self.pairing_json.clone());
                                self.logs.lock().unwrap().push("📋 Copied JSON to clipboard".into());
                            }
                        });
                    });
                });

            ui.add_space(10.0);

            // Connection Info & Settings Card
            egui::Frame::none()
                .fill(Color32::from_rgb(18, 20, 29))
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 43, 58)))
                .rounding(12.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    egui::Grid::new("info_grid")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("Relay:").color(Color32::from_rgb(148, 163, 184)));
                            ui.label(RichText::new(&self.config.relay).strong().color(Color32::WHITE));
                            ui.end_row();

                            ui.label(RichText::new("Room ID:").color(Color32::from_rgb(148, 163, 184)));
                            ui.label(RichText::new(&self.config.room_id[..18]).monospace().color(Color32::from_rgb(56, 189, 248)));
                            ui.end_row();

                            ui.label(RichText::new("Shell:").color(Color32::from_rgb(148, 163, 184)));
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("shell_select")
                                    .selected_text(&self.selected_shell)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.selected_shell, detect_best_shell(), "PowerShell");
                                        ui.selectable_value(&mut self.selected_shell, "cmd.exe".into(), "CMD");
                                    });
                            });
                            ui.end_row();
                        });

                    ui.add_space(4.0);
                    if ui.button("🔄 Reset Room & Regenerate Keys").clicked() {
                        self.regenerate_pairing(ctx);
                    }
                });

            ui.add_space(8.0);

            // Activity Log
            ui.label(RichText::new("SESSION ACTIVITY").size(11.0).strong().color(Color32::from_rgb(148, 163, 184)));
            egui::Frame::none()
                .fill(Color32::from_rgb(10, 11, 16))
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(26, 30, 42)))
                .rounding(8.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(80.0)
                        .auto_shrink([false; 2])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            let logs = self.logs.lock().unwrap();
                            for log in logs.iter().rev().take(15).rev() {
                                ui.label(RichText::new(log).size(10.5).monospace().color(Color32::from_rgb(203, 213, 225)));
                            }
                        });
                });
        });
    }
}

// ==============================================================================
// Headless CLI fallback mode (for scripts / servers)
// ==============================================================================

fn run_cli_mode(config: HostConfig) -> anyhow::Result<()> {
    let keypair = Arc::new(config.get_keypair());
    let auth_token = config.get_auth_token();
    let pairing = config.get_pairing_payload();
    let pairing_url = pairing.to_url()?;

    println!("OmniLink CLI Mode Active");
    println!("Pairing URL: {}", pairing_url);
    println!("Room UUID  : {}", config.room_id);

    let (session, _killswitch) = TunnelHostSession::new(
        config.relay,
        config.room_id,
        keypair,
        auth_token,
        config.pin,
        config.shell,
    );

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(session.run())
}
