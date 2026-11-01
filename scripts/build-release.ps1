# ==============================================================================
# PCTunnel Build Release & Distribution Packaging Script
# ==============================================================================

$ErrorActionPreference = "Stop"

Write-Host "[+] Building PCTunnel in release mode..." -ForegroundColor Cyan
cargo build --release --workspace

$distDir = "dist"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
}

Write-Host "[+] Packaging Portable Windows Bundle..." -ForegroundColor Yellow
$zipFile = "$distDir\pctunnel-windows-x64.zip"
if (Test-Path $zipFile) { Remove-Item $zipFile -Force }

Compress-Archive -Path @(
    "target\release\pctunnel-app.exe",
    "target\release\pctunnel-mcp.exe",
    "scripts\install.ps1",
    "README.md"
) -DestinationPath $zipFile -Force

Write-Host "[+] Done! Output available at: $zipFile" -ForegroundColor Green
