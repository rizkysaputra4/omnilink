#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use clap::Parser;
use eframe::egui::{self, Color32, RichText, Vec2};
use omnilink_core::{
    spawn_terminal_instance, SharedBroadcaster, SharedTerminalRegistry, TunnelHostSession,
};
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

fn load_app_icon() -> Option<egui::IconData> {
    let icon_bytes = include_bytes!("../../../assets/icon.png");
    if let Ok(img) = image::load_from_memory(icon_bytes) {
        let rgba = img.into_rgba8();
        let (width, height) = rgba.dimensions();
        Some(egui::IconData {
            rgba: rgba.into_raw(),
            width,
            height,
        })
    } else {
        None
    }
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
        return run_cli_mode(config);
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 720.0])
        .with_min_inner_size([880.0, 640.0])
        .with_title("OmniLink - Secure Zero-Trust Terminal Bridge")
        .with_active(true);

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let res = eframe::run_native(
        "OmniLink Desktop",
        options,
        Box::new(|cc| Ok(Box::new(OmniLinkGui::new(cc, config)))),
    );
    let _ = std::fs::write("gui_err.txt", format!("{:?}", res));
    res.map_err(|e| anyhow::anyhow!("GUI Error: {}", e))
}

// ==============================================================================
// GUI Application State with Mirrored Terminal
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

    // Mirrored Multi-Terminal State
    terminals: SharedTerminalRegistry,
    broadcaster: SharedBroadcaster,
    active_term_id: u32,
    pc_command_input: String,
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

        let terminals: SharedTerminalRegistry = Arc::new(Mutex::new(HashMap::new()));
        let broadcaster: SharedBroadcaster = Arc::new(Mutex::new(None));

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
            terminals,
            broadcaster,
            active_term_id: 0,
            pc_command_input: String::new(),
        };

        app.start_service();
        app
    }

    fn generate_qr_texture(ctx: &egui::Context, url: &str) -> Option<egui::TextureHandle> {
        let qr = QrCode::new(url.as_bytes()).ok()?;
        let width = qr.width();
        let border_modules = 5;
        let scale = 6;
        let total_modules = width + border_modules * 2;
        let total_px = total_modules * scale;
        let mut pixels = vec![Color32::WHITE; total_px * total_px];

        for y in 0..width {
            for x in 0..width {
                if qr[(x, y)] == qrcode::Color::Dark {
                    let start_px = (x + border_modules) * scale;
                    let start_py = (y + border_modules) * scale;
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = start_px + dx;
                            let py = start_py + dy;
                            pixels[py * total_px + px] = Color32::BLACK;
                        }
                    }
                }
            }
        }

        let image = egui::ColorImage {
            size: [total_px, total_px],
            pixels,
        };

        Some(ctx.load_texture("omnilink_qr", image, egui::TextureOptions::LINEAR))
    }

    fn start_service(&mut self) {
        if self.is_running {
            return;
        }

        let host_key = Arc::new(self.config.get_keypair());
        let auth_token = self.config.get_auth_token();
        let pin = self.config.pin.clone();
        let room_id = self.config.room_id.clone();
        let relay_url = self.config.relay.clone();
        let shell = self.selected_shell.clone();

        let (session, killswitch) = TunnelHostSession::with_terminals(
            relay_url,
            room_id,
            host_key,
            auth_token,
            pin,
            shell.clone(),
            self.terminals.clone(),
            self.broadcaster.clone(),
        );

        // Pre-spawn terminal 0 so PC GUI immediately displays a ready shell!
        let _ = spawn_terminal_instance(
            0,
            shell,
            &self.terminals,
            &self.broadcaster,
            &killswitch,
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
                    while !killswitch.load(Ordering::SeqCst) {
                        logs.lock().unwrap().push("🟢 Listening for incoming connections...".into());
                        match session.run().await {
                            Ok(_) => {
                                logs.lock().unwrap().push("Session finished. Ready for next connection...".into());
                            }
                            Err(e) => {
                                if !killswitch.load(Ordering::SeqCst) {
                                    warn!("Session notice: {}", e);
                                    logs.lock().unwrap().push(format!("⚠️ Connection closed: {}. Waiting...", e));
                                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                }
                            }
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

    fn send_keystroke_to_active(&self, input: &[u8]) {
        if let Some(term) = self.terminals.lock().unwrap().get(&self.active_term_id) {
            let _ = term.write_input(input);
        }
    }
}

impl eframe::App for OmniLinkGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                // ==========================================================
                // Left Column: Control Panel & Pairing Card (Width: ~340px)
                // ==========================================================
                let left = &mut columns[0];

                left.horizontal(|ui| {
                    ui.label(RichText::new("OMNILINK").size(18.0).strong().color(Color32::WHITE));
                    ui.label(RichText::new("HOST").size(10.0).strong().background_color(Color32::from_rgb(30, 41, 59)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.is_running {
                            ui.label(RichText::new("● ONLINE").size(11.0).strong().color(Color32::from_rgb(16, 185, 129)));
                        } else {
                            ui.label(RichText::new("● STOPPED").size(11.0).strong().color(Color32::from_rgb(239, 68, 68)));
                        }
                    });
                });

                left.add_space(8.0);

                let btn_size = Vec2::new(left.available_width(), 40.0);
                if self.is_running {
                    let stop_btn = egui::Button::new(
                        RichText::new("⏹  STOP SERVICE")
                            .size(13.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(220, 38, 38))
                    .min_size(btn_size);

                    if left.add(stop_btn).clicked() {
                        self.stop_service();
                    }
                } else {
                    let start_btn = egui::Button::new(
                        RichText::new("▶  START SERVICE")
                            .size(13.0)
                            .strong()
                            .color(Color32::BLACK),
                    )
                    .fill(Color32::from_rgb(0, 242, 254))
                    .min_size(btn_size);

                    if left.add(start_btn).clicked() {
                        self.start_service();
                    }
                }

                left.add_space(8.0);

                // QR Code Pairing Frame
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 20, 29))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 43, 58)))
                    .rounding(10.0)
                    .inner_margin(10.0)
                    .show(left, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("PAIR WITH MOBILE APP").size(13.0).strong().color(Color32::WHITE));
                            ui.label(
                                RichText::new("Scan once from OmniLink Android App")
                                    .size(10.0)
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );

                            ui.add_space(6.0);

                            egui::Frame::none()
                                .fill(Color32::WHITE)
                                .rounding(6.0)
                                .inner_margin(4.0)
                                .show(ui, |ui| {
                                    if let Some(texture) = &self.qr_texture {
                                        ui.image((texture.id(), Vec2::new(170.0, 170.0)));
                                    }
                                });

                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                if ui.button("📋 1-Click Link").clicked() {
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

                left.add_space(6.0);

                // Room Config
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 20, 29))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 43, 58)))
                    .rounding(8.0)
                    .inner_margin(8.0)
                    .show(left, |ui| {
                        egui::Grid::new("info_grid")
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Room ID:").size(10.0).color(Color32::from_rgb(148, 163, 184)));
                                ui.label(RichText::new(&self.config.room_id[..18]).size(10.0).monospace().color(Color32::from_rgb(56, 189, 248)));
                                ui.end_row();

                                ui.label(RichText::new("Shell:").size(10.0).color(Color32::from_rgb(148, 163, 184)));
                                egui::ComboBox::from_id_salt("shell_select")
                                    .selected_text(&self.selected_shell)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.selected_shell, detect_best_shell(), "PowerShell");
                                        ui.selectable_value(&mut self.selected_shell, "cmd.exe".into(), "CMD");
                                    });
                                ui.end_row();
                            });

                        ui.add_space(4.0);
                        if ui.button("🔄 Reset Room & Key").clicked() {
                            self.regenerate_pairing(ctx);
                        }
                    });

                left.add_space(6.0);

                // Activity Log
                left.label(RichText::new("HOST ACTIVITY").size(10.0).strong().color(Color32::from_rgb(148, 163, 184)));
                egui::Frame::none()
                    .fill(Color32::from_rgb(10, 11, 16))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(26, 30, 42)))
                    .rounding(6.0)
                    .inner_margin(6.0)
                    .show(left, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(80.0)
                            .auto_shrink([false; 2])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                let logs = self.logs.lock().unwrap();
                                for log in logs.iter().rev().take(10).rev() {
                                    ui.label(RichText::new(log).size(9.5).monospace().color(Color32::from_rgb(203, 213, 225)));
                                }
                            });
                    });

                // ==========================================================
                // Right Column: Mirrored Multi-Terminal Dashboard
                // ==========================================================
                let right = &mut columns[1];

                // Terminal Tab Bar (Mirrors active terminals from phone or PC)
                let term_ids: Vec<u32> = {
                    let map = self.terminals.lock().unwrap();
                    let mut ids: Vec<u32> = map.keys().copied().collect();
                    ids.sort();
                    ids
                };

                right.horizontal(|ui| {
                    ui.label(RichText::new("MIRRORED TERMINAL").size(13.0).strong().color(Color32::from_rgb(0, 242, 254)));
                    ui.add_space(4.0);

                    for id in &term_ids {
                        let is_active = *id == self.active_term_id;
                        let label = format!(">_ Terminal {}", id + 1);
                        let tab_btn = egui::Button::new(
                            RichText::new(label)
                                .size(11.0)
                                .color(if is_active { Color32::WHITE } else { Color32::from_rgb(148, 163, 184) })
                        )
                        .fill(if is_active { Color32::from_rgb(30, 41, 59) } else { Color32::from_rgb(18, 20, 29) });

                        if ui.add(tab_btn).clicked() {
                            self.active_term_id = *id;
                        }
                    }

                    // [+] Add new terminal tab
                    if ui.button(RichText::new("+ New").size(11.0).color(Color32::from_rgb(0, 242, 254))).clicked() {
                        let next_id = term_ids.iter().max().map_or(0, |m| m + 1);
                        if let Some(ks) = &self.active_killswitch {
                            let _ = spawn_terminal_instance(
                                next_id,
                                self.selected_shell.clone(),
                                &self.terminals,
                                &self.broadcaster,
                                ks,
                            );
                            self.active_term_id = next_id;
                        }
                    }

                    // [+ 🚀 AGY]
                    if ui.button(RichText::new("+ 🚀 AGY").size(11.0).strong().color(Color32::from_rgb(0, 242, 254))).clicked() {
                        let next_id = term_ids.iter().max().map_or(0, |m| m + 1);
                        if let Some(ks) = &self.active_killswitch {
                            let _ = spawn_terminal_instance(
                                next_id,
                                self.selected_shell.clone(),
                                &self.terminals,
                                &self.broadcaster,
                                ks,
                            );
                            self.active_term_id = next_id;
                            let terms = self.terminals.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(350));
                                if let Some(term) = terms.lock().unwrap().get(&next_id) {
                                    let _ = term.write_input(b"agy --dangerously-skip-permissions\r");
                                }
                            });
                        }
                    }

                    // [+ 🤖 Claude]
                    if ui.button(RichText::new("+ 🤖 Claude").size(11.0).strong().color(Color32::from_rgb(245, 158, 11))).clicked() {
                        let next_id = term_ids.iter().max().map_or(0, |m| m + 1);
                        if let Some(ks) = &self.active_killswitch {
                            let _ = spawn_terminal_instance(
                                next_id,
                                self.selected_shell.clone(),
                                &self.terminals,
                                &self.broadcaster,
                                ks,
                            );
                            self.active_term_id = next_id;
                            let terms = self.terminals.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(350));
                                if let Some(term) = terms.lock().unwrap().get(&next_id) {
                                    let _ = term.write_input(b"claude --dangerously-skip-permissions\r");
                                }
                            });
                        }
                    }
                });

                right.add_space(4.0);

                // Mirrored Monospace Output Area
                egui::Frame::none()
                    .fill(Color32::from_rgb(5, 7, 10))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 59)))
                    .rounding(8.0)
                    .inner_margin(10.0)
                    .show(right, |ui| {
                        let available_height = ui.available_height() - 84.0;
                        egui::ScrollArea::vertical()
                            .max_height(available_height.max(300.0))
                            .auto_shrink([false; 2])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                let display_text = {
                                    let map = self.terminals.lock().unwrap();
                                    if let Some(term) = map.get(&self.active_term_id) {
                                        term.display_text.lock().unwrap().clone()
                                    } else {
                                        "No terminal session active. Click '+ New' above to spawn.".into()
                                    }
                                };

                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&display_text)
                                            .font(egui::FontId::monospace(11.5))
                                            .color(Color32::from_rgb(226, 232, 240)),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                            });
                    });

                right.add_space(4.0);

                // Quick Keystrokes Toolbar
                right.horizontal(|ui| {
                    if ui.button(RichText::new("🚀 AGY").size(10.0).strong().color(Color32::from_rgb(0, 242, 254))).clicked() {
                        self.send_keystroke_to_active(b"agy --dangerously-skip-permissions\r");
                    }
                    if ui.button(RichText::new("🤖 Claude").size(10.0).strong().color(Color32::from_rgb(245, 158, 11))).clicked() {
                        self.send_keystroke_to_active(b"claude --dangerously-skip-permissions\r");
                    }
                    if ui.add(
                        egui::Button::new(RichText::new("⏎ ENTER").size(10.0).strong().color(Color32::BLACK))
                            .fill(Color32::from_rgb(0, 242, 254))
                    ).clicked() {
                        self.send_keystroke_to_active(b"\r");
                    }
                    if ui.button(RichText::new("ESC").size(10.0).color(Color32::from_rgb(245, 158, 11))).clicked() {
                        self.send_keystroke_to_active(b"\x1b");
                    }
                    if ui.button(RichText::new("^C").size(10.0).color(Color32::from_rgb(239, 68, 68))).clicked() {
                        self.send_keystroke_to_active(b"\x03");
                    }
                    if ui.button(RichText::new("TAB").size(10.0)).clicked() {
                        self.send_keystroke_to_active(b"\t");
                    }
                    if ui.button(RichText::new("▲").size(10.0)).clicked() {
                        self.send_keystroke_to_active(b"\x1b[A");
                    }
                    if ui.button(RichText::new("▼").size(10.0)).clicked() {
                        self.send_keystroke_to_active(b"\x1b[B");
                    }
                });

                right.add_space(4.0);

                // PC Keystroke / Command Direct Input Bar
                right.horizontal(|ui| {
                    ui.label(RichText::new("❯").size(14.0).strong().color(Color32::from_rgb(0, 242, 254)));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.pc_command_input)
                            .hint_text("Type PC command or input here (mirrors live to phone)...")
                            .desired_width(ui.available_width() - 60.0),
                    );

                    let send_clicked = ui.add(
                        egui::Button::new(RichText::new("Send").size(11.0).strong().color(Color32::BLACK))
                            .fill(Color32::from_rgb(0, 242, 254))
                    ).clicked();

                    if send_clicked || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        let mut cmd = self.pc_command_input.clone();
                        cmd.push('\r');
                        self.send_keystroke_to_active(cmd.as_bytes());
                        self.pc_command_input.clear();
                        response.request_focus();
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
