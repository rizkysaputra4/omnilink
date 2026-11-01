# ==============================================================================
# PCTunnel Windows One-Click Installer (Minimum Steps To Run)
# Run via: irm https://findbeatradar.com/install.ps1 | iex
# Or: .\scripts\install.ps1
# ==============================================================================

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host "    🔒 Installing PCTunnel (Zero-Trust Terminal)      " -ForegroundColor Green
Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host ""

$installDir = "$env:LOCALAPPDATA\Programs\PCTunnel"
$binPath = "$installDir\pctunnel-app.exe"
$mcpPath = "$installDir\pctunnel-mcp.exe"

# 1. Create install directory
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

# 2. Check if running in repo or remote download
$currentScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path -ErrorAction SilentlyContinue
$localRelease = "$currentScriptDir\..\target\release\pctunnel-app.exe"

if (Test-Path $localRelease) {
    Write-Host "[+] Installing from local build..." -ForegroundColor Yellow
    Copy-Item "$currentScriptDir\..\target\release\pctunnel-app.exe" -Destination $binPath -Force
    if (Test-Path "$currentScriptDir\..\target\release\pctunnel-mcp.exe") {
        Copy-Item "$currentScriptDir\..\target\release\pctunnel-mcp.exe" -Destination $mcpPath -Force
    }
} else {
    Write-Host "[+] Building local release binary via Cargo..." -ForegroundColor Yellow
    cargo build --release -p pctunnel-app -p pctunnel-mcp
    Copy-Item "target\release\pctunnel-app.exe" -Destination $binPath -Force
    Copy-Item "target\release\pctunnel-mcp.exe" -Destination $mcpPath -Force
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
$shortcutFile = "$desktopPath\PCTunnel.lnk"
$wscript = New-Object -ComObject WScript.Shell
$shortcut = $wscript.CreateShortcut($shortcutFile)
$shortcut.TargetPath = $binPath
$shortcut.Description = "PCTunnel Secure Zero-Trust Terminal"
$shortcut.WorkingDirectory = $installDir
$shortcut.Save()

# 5. Create Start Menu Shortcut
$startMenuPath = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\PCTunnel"
if (-not (Test-Path $startMenuPath)) {
    New-Item -ItemType Directory -Path $startMenuPath -Force | Out-Null
}
$startMenuShortcut = $wscript.CreateShortcut("$startMenuPath\PCTunnel.lnk")
$startMenuShortcut.TargetPath = $binPath
$startMenuShortcut.Description = "PCTunnel Secure Zero-Trust Terminal"
$startMenuShortcut.WorkingDirectory = $installDir
$startMenuShortcut.Save()

Write-Host ""
Write-Host "=====================================================" -ForegroundColor Green
Write-Host "  ✅ PCTunnel Installed Successfully!               " -ForegroundColor Green
Write-Host "  Desktop Shortcut: Created                          " -ForegroundColor White
Write-Host "  Command Line:     Type 'pctunnel-app' in terminal  " -ForegroundColor White
Write-Host "=====================================================" -ForegroundColor Green
Write-Host ""

# Ask to launch immediately
$response = Read-Host "Would you like to start PCTunnel now? (Y/n)"
if ($response -ne 'n' -and $response -ne 'N') {
    Start-Process -FilePath $binPath
}
