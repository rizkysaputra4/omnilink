# ==============================================================================
# OmniLink Windows One-Click Installer (Minimum Steps To Run)
# Run via: irm https://raw.githubusercontent.com/rizkysaputra4/omnilink/main/scripts/install.ps1 | iex
# Or: .\scripts\install.ps1
# ==============================================================================

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host "    🔒 Installing OmniLink (Zero-Trust Terminal)      " -ForegroundColor Green
Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host ""

$installDir = "$env:LOCALAPPDATA\Programs\OmniLink"
$binPath = "$installDir\omnilink-app.exe"
$mcpPath = "$installDir\omnilink-mcp.exe"

# 1. Create install directory
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

# 2. Check if running in repo or remote download
$currentScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path -ErrorAction SilentlyContinue
$localRelease = "$currentScriptDir\..\target\release\omnilink-app.exe"

if (Test-Path $localRelease) {
    Write-Host "[+] Installing from local build..." -ForegroundColor Yellow
    Copy-Item "$currentScriptDir\..\target\release\omnilink-app.exe" -Destination $binPath -Force
    if (Test-Path "$currentScriptDir\..\target\release\omnilink-mcp.exe") {
        Copy-Item "$currentScriptDir\..\target\release\omnilink-mcp.exe" -Destination $mcpPath -Force
    }
} else {
    Write-Host "[+] Building local release binary via Cargo..." -ForegroundColor Yellow
    cargo build --release -p omnilink-app -p omnilink-mcp
    Copy-Item "target\release\omnilink-app.exe" -Destination $binPath -Force
    Copy-Item "target\release\omnilink-mcp.exe" -Destination $mcpPath -Force
}

# 3. Add to User PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    Write-Host "[+] Adding $installDir to User PATH..." -ForegroundColor Cyan
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    $env:Path = "$env:Path;$installDir"
}

# 4. Create Desktop Shortcut
$desktopPath = [Environment]::GetFolderPath("Desktop")
$shortcutFile = "$desktopPath\OmniLink.lnk"
$wscript = New-Object -ComObject WScript.Shell
$shortcut = $wscript.CreateShortcut($shortcutFile)
$shortcut.TargetPath = $binPath
$shortcut.Description = "OmniLink Secure Zero-Trust Terminal"
$shortcut.WorkingDirectory = $installDir
$shortcut.Save()

# 5. Create Start Menu Shortcut
$startMenuPath = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\OmniLink"
if (-not (Test-Path $startMenuPath)) {
    New-Item -ItemType Directory -Path $startMenuPath -Force | Out-Null
}
$startMenuShortcut = $wscript.CreateShortcut("$startMenuPath\OmniLink.lnk")
$startMenuShortcut.TargetPath = $binPath
$startMenuShortcut.Description = "OmniLink Secure Zero-Trust Terminal"
$startMenuShortcut.WorkingDirectory = $installDir
$startMenuShortcut.Save()

Write-Host ""
Write-Host "=====================================================" -ForegroundColor Green
Write-Host "  ✅ OmniLink Installed Successfully!                " -ForegroundColor Green
Write-Host "  Desktop Shortcut: Created                          " -ForegroundColor White
Write-Host "  Command Line:     Type 'omnilink-app' in terminal  " -ForegroundColor White
Write-Host "=====================================================" -ForegroundColor Green
Write-Host ""

# Ask to launch immediately
$response = Read-Host "Would you like to start OmniLink now? (Y/n)"
if ($response -ne 'n' -and $response -ne 'N') {
    Start-Process -FilePath $binPath
}
