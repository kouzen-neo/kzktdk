# KZKT-DK (kzktdk) Universal Installer for Windows PowerShell
# Usage: irm https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "kouzen-neo/kzktdk"
$FallbackTag = "v0.2.1"

# 1. Architecture Check
if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Error "[-] KZKT-DK currently requires a 64-bit Windows operating system (x86_64)."
    exit 1
}

Write-Host "[*] Detected Windows x86_64 platform." -ForegroundColor Cyan

# 2. Resolve latest version tag
Write-Host "[*] Fetching latest release info..." -ForegroundColor Cyan
$Tag = $null
try {
    $ReleaseInfo = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing -TimeoutSec 10
    if ($ReleaseInfo.tag_name) {
        $Tag = $ReleaseInfo.tag_name
    }
} catch {
    # Fallback if GitHub API rate-limited
    $Tag = $FallbackTag
}

if (-not $Tag) {
    $Tag = $FallbackTag
}

$AssetName = "kzktdk-x86_64-pc-windows-msvc.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Tag/$AssetName"

Write-Host "[*] Installing KZKT-DK $Tag..." -ForegroundColor Cyan
Write-Host "[*] Download URL: $DownloadUrl" -ForegroundColor Gray

# 3. Setup Temp & Destination Directories
$TempZip = Join-Path $env:TEMP "$AssetName"
$TempExtract = Join-Path $env:TEMP "kzktdk_extract_$(Get-Random)"

$InstallDir = Join-Path $env:LOCALAPPDATA "kzktdk"
$BinDir = Join-Path $InstallDir "bin"
$ModelsDir = Join-Path $InstallDir "models"
$FontsDir = Join-Path $InstallDir "fonts"

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
New-Item -ItemType Directory -Force -Path $ModelsDir | Out-Null
New-Item -ItemType Directory -Force -Path $FontsDir | Out-Null

try {
    # 4. Download and Extract
    Write-Host "[*] Downloading package..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempZip -UseBasicParsing

    Write-Host "[*] Extracting..." -ForegroundColor Cyan
    Expand-Archive -Path $TempZip -DestinationPath $TempExtract -Force

    # Locate files in extract directory (or nested folder)
    $SourceDir = $TempExtract
    $NestedDir = Join-Path $TempExtract "kzktdk-x86_64-pc-windows-msvc"
    if (Test-Path $NestedDir) {
        $SourceDir = $NestedDir
    }

    # 5. Copy Binary and Assets
    Write-Host "[*] Installing binary to $BinDir\kzktdk.exe..." -ForegroundColor Cyan
    $ExePath = Join-Path $SourceDir "kzktdk.exe"
    if (-not (Test-Path $ExePath)) {
        # Search recursively if needed
        $FoundExe = Get-ChildItem -Path $TempExtract -Filter "kzktdk.exe" -Recurse | Select-Object -First 1
        if ($FoundExe) {
            $ExePath = $FoundExe.FullName
        } else {
            throw "Executable kzktdk.exe not found in archive."
        }
    }
    Copy-Item -Path $ExePath -Destination (Join-Path $BinDir "kzktdk.exe") -Force

    # Copy models and fonts
    $ExtractModels = Join-Path $SourceDir "models"
    if (Test-Path $ExtractModels) {
        Write-Host "[*] Installing models to $ModelsDir..." -ForegroundColor Cyan
        Copy-Item -Path "$ExtractModels\*" -Destination $ModelsDir -Recurse -Force
    }

    $ExtractFonts = Join-Path $SourceDir "fonts"
    if (Test-Path $ExtractFonts) {
        Write-Host "[*] Installing fonts to $FontsDir..." -ForegroundColor Cyan
        Copy-Item -Path "$ExtractFonts\*" -Destination $FontsDir -Recurse -Force
    }

    # 6. Add to PATH if missing
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", [EnvironmentVariableTarget]::User)
    if ($UserPath -split ";" -notcontains $BinDir) {
        Write-Host "[*] Adding $BinDir to User PATH..." -ForegroundColor Cyan
        $NewPath = "$UserPath;$BinDir"
        [Environment]::SetEnvironmentVariable("PATH", $NewPath, [EnvironmentVariableTarget]::User)
    }

    # Update current session PATH
    if ($env:PATH -split ";" -notcontains $BinDir) {
        $env:PATH = "$env:PATH;$BinDir"
    }

    Write-Host ""
    Write-Host "=========================================================" -ForegroundColor Green
    Write-Host "  KZKT-DK (kzktdk) successfully installed!" -ForegroundColor Green
    Write-Host "=========================================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "[+] Installed binary: $BinDir\kzktdk.exe" -ForegroundColor Green
    Write-Host "[+] Data directory:   $InstallDir" -ForegroundColor Green
    Write-Host ""
    Write-Host "Run 'kzktdk --help' in a new terminal window to get started!" -ForegroundColor Yellow
} finally {
    # Cleanup
    if (Test-Path $TempZip) { Remove-Item -Force $TempZip -ErrorAction SilentlyContinue }
    if (Test-Path $TempExtract) { Remove-Item -Force -Recurse $TempExtract -ErrorAction SilentlyContinue }
}
