# ==============================================================================
# OmniLink Build Release & Distribution Packaging Script
# ==============================================================================

$ErrorActionPreference = "Stop"

Write-Host "[+] Building OmniLink in release mode..." -ForegroundColor Cyan
cargo build --release --workspace

$distDir = "dist"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
}

Write-Host "[+] Packaging Portable Windows Bundle..." -ForegroundColor Yellow
$zipFile = "$distDir\omnilink-windows-x64.zip"
if (Test-Path $zipFile) { Remove-Item $zipFile -Force }

Compress-Archive -Path @(
    "target\release\omnilink-app.exe",
    "target\release\omnilink-mcp.exe",
    "scripts\install.ps1",
    "README.md"
) -DestinationPath $zipFile -Force

Write-Host "[+] Done! Output available at: $zipFile" -ForegroundColor Green
