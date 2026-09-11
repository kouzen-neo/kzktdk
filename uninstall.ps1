# KZKT-DK (kzktdk) Uninstaller for Windows PowerShell
# Usage: irm https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/uninstall.ps1 | iex

$ErrorActionPreference = "SilentlyContinue"

Write-Host "[*] Uninstalling KZKT-DK (kzktdk)..." -ForegroundColor Cyan

$InstallDir = Join-Path $env:LOCALAPPDATA "kzktdk"
$BinDir = Join-Path $InstallDir "bin"

# 1. Remove Install Directory
if (Test-Path $InstallDir) {
    Remove-Item -Path $InstallDir -Recurse -Force
    Write-Host "[+] Removed directory: $InstallDir" -ForegroundColor Green
} else {
    Write-Host "[-] No installation found at $InstallDir." -ForegroundColor Gray
}

# 2. Clean User PATH environment variable
$UserPath = [Environment]::GetEnvironmentVariable("PATH", [EnvironmentVariableTarget]::User)
if ($UserPath -and ($UserPath -split ";" -contains $BinDir)) {
    $NewPath = ($UserPath -split ";" | Where-Object { $_ -ne $BinDir }) -join ";"
    [Environment]::SetEnvironmentVariable("PATH", $NewPath, [EnvironmentVariableTarget]::User)
    Write-Host "[+] Removed $BinDir from User PATH." -ForegroundColor Green
}

# 3. Clean Cache if present
$CacheDir = Join-Path $env:LOCALAPPDATA "kzktdk_cache"
if (Test-Path $CacheDir) {
    Remove-Item -Path $CacheDir -Recurse -Force
    Write-Host "[+] Removed cache directory: $CacheDir" -ForegroundColor Green
}

Write-Host ""
Write-Host "=========================================================" -ForegroundColor Green
Write-Host "  KZKT-DK has been successfully uninstalled from Windows." -ForegroundColor Green
Write-Host "=========================================================" -ForegroundColor Green
