# Builds Nimby with the GNU Rust toolchain plus a local MinGW-w64.
# Usage:  powershell -File scripts\build.ps1 [cargo args...]
$ErrorActionPreference = 'Stop'

$mingw = 'D:\dev\tools\mingw64'
if (-not (Test-Path "$mingw\bin\gcc.exe")) {
    throw "MinGW-w64 not found at $mingw. Download winlibs (x86_64 posix seh) and extract it there."
}

$env:PATH = "$mingw\bin;$mingw\x86_64-w64-mingw32\bin;$env:PATH"
$env:CC = "$mingw\bin\gcc.exe"
$env:CXX = "$mingw\bin\g++.exe"
$env:AR = "$mingw\bin\ar.exe"

# The system drive can be short on temp space; keep build scratch local.
$tmp = 'D:\dev\tmp'
New-Item -ItemType Directory -Force $tmp | Out-Null
$env:TEMP = $tmp
$env:TMP = $tmp

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    if ($args.Count -eq 0) {
        cargo build
    } else {
        & cargo @args
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
