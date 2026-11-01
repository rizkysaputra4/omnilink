# 🔒 OmniLink

> **Zero-Trust, End-to-End Encrypted Remote Terminal Bridge**  
> Access your local Windows terminal securely from your Android phone without opening router ports, without SSH passwords, and without giving anyone shell access to your VPS mediator.

---

## 🌟 Key Architectural Features

- **Blind Rendezvous Relay**: The VPS relay (`relay.findbeatradar.com`) operates as a dumb binary forwarder. It has **no shell, no user accounts, and zero file access**. Even if the VPS is compromised, the attacker cannot read or inject terminal data.
- **256-Bit Cryptographic Proof-of-Possession**: Knowing or guessing someone's Room ID provides **ZERO access**. The host PC strictly requires an HMAC-SHA256 proof computed from the high-entropy 256-bit token embedded in the physical QR code.
- **End-to-End Encryption (E2EE)**: Handshake via **X25519 (ECDH)** and continuous symmetric encryption via **ChaCha20-Poly1305** with monotonic nonces to prevent replay attacks.
- **Direct SSH Mode**: Seamless 1-tap switcher to connect directly to local LAN/WiFi/VPN OpenSSH servers without going through the relay.
- **Hardware Killswitch**: Stop button in the app instantly kills all background ConPTY processes, closes sockets, and wipes session memory.
- **Native Windows ConPTY**: Directly utilizes Windows Pseudo Console API (`CreatePseudoConsole`), running as your standard unprivileged Windows user.
- **AI Agent Integration (MCP)**: Local loopback Model Context Protocol server (`127.0.0.1`) for Claude Desktop and Antigravity CLI.

---

## 🚀 3-Second Quickstart (Minimum Steps)

### 1. Windows PC (Host)
Run this single command in PowerShell:
```powershell
irm https://raw.githubusercontent.com/rizkysaputra4/omnilink/main/scripts/install.ps1 | iex
```
*Or download and double-click `omnilink-app.exe` from Releases.*

A sleek modern dashboard with a QR code will immediately appear on your PC terminal screen:
```
╭─────────────────────────────────────────────────────────────────────╮
│   OMNILINK  │  ● HOST ACTIVE  (Zero-Trust E2EE ConPTY Bridge)       │
╰─────────────────────────────────────────────────────────────────────╯

  [ 1. SCAN TO CONNECT (Zero-Config / Remote) ]
  Scan this QR code from the OmniLink Android App:

    [█▀▀▀▀▀█ ▄▄ ▄▄ █▀▀▀▀▀█]
    [█ ███ █ █ ▄ █ █ ███ █]
    [█ ▀▀▀ █ ▄▀▄ █ █ ▀▀▀ █]
    ...

  ╭─ Connection Details ───────────────────────────────────────────────╮
  │  Relay Host   : wss://relay.findbeatradar.com
  │  Room UUID    : 550e8400-e29b-41d4-a716-446655440000
  │  Auth Token   : a3f89b1c4e20... (256-bit cryptosecret)
  │  Target Shell : C:\Program Files\PowerShell\7\pwsh.exe
  │  Security     : ChaCha20-Poly1305 + X25519 (Proof-Gated)
  ╰────────────────────────────────────────────────────────────────────╯
```

### 2. Android Smartphone
1. Open **OmniLink** app on your phone.
2. Tap **Pair & Connect** and scan the QR code from the PC screen (or use Direct SSH).
3. **Connected!** Your encrypted terminal session is active immediately.

---

## ☁️ Deploying Relay on VPS (`findbeatradar.com`)

To host the blind forwarder on your own VPS subdomain:

1. **DNS Setup**:
   - Add an **A Record** in your DNS management:
     - Name: `relay.findbeatradar.com`
     - Value: `<Your-VPS-IP>`
2. **Deploy with 1 Command**:
   ```bash
   sudo ./deploy/deploy-relay.sh relay.findbeatradar.com
   ```
   *This automatically sets up Docker, UFW firewall (allowing only 80, 443, 22), OmniLink Relay, and Caddy with automatic Let's Encrypt SSL!*

---

## 🤖 MCP Server Setup for AI Agents (Claude / Antigravity)

Add this configuration to your `claude_desktop_config.json` or Antigravity config:
```json
{
  "mcpServers": {
    "omnilink": {
      "command": "omnilink-mcp.exe"
    }
  }
}
```

Now your AI assistant can notify your phone whenever a long-running compile or analysis task finishes!

---

## 🛠️ Project Structure

```
omnilink/
├── crates/
│   ├── omnilink-proto/    # E2EE crypto (X25519 + ChaCha20Poly1305), auth-proof & framing
│   ├── omnilink-relay/    # Blind async WebSocket rendezvous forwarder
│   ├── omnilink-core/     # Windows ConPTY engine, session manager & whitelist
│   ├── omnilink-app/      # Host CLI & GUI wrapper with QR generator & Killswitch
│   └── omnilink-mcp/      # Local loopback MCP server for AI agents
├── android/               # Native Android Client (Kotlin + Jetpack Compose + CameraX)
├── deploy/                # Dockerfile, docker-compose.yml, Caddyfile, deploy-relay.sh
├── scripts/               # install.ps1, build-release.ps1
└── PLAN.md                # System Architecture & Threat Model Documentation
```

---

## 📜 License
Dual-licensed under MIT or Apache-2.0.
