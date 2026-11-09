# Nimby build helper.
#
# Usage:
#   powershell -File scripts\build.ps1 [cargo args...]
#
# With no arguments this runs "cargo build". Every argument is passed to cargo
# unchanged (for example: test, clippy --all-targets -- -D warnings,
# build --release) and cargo's exit code is propagated. The script may be
# started from any directory; cargo always runs with the repository root as
# its working directory.
#
# Environment variables:
#   NIMBY_MINGW  Directory that contains bin\gcc.exe (a MinGW-w64 toolchain,
#                for example C:\msys64\mingw64). Only needed when gcc/ar are
#                not already on PATH and the Rust host toolchain is
#                x86_64-pc-windows-gnu. Sets PATH, CC, CXX and AR.
#   NIMBY_TMP    Directory used as TEMP/TMP for the build, for machines whose
#                system drive is short on temp space. Created when missing.
#                When unset, TEMP and TMP are left untouched.
#
# MSVC users need nothing: with the default x86_64-pc-windows-msvc host and
# Visual Studio Build Tools installed the script simply runs cargo.

$ErrorActionPreference = 'Stop'

function Test-ToolOnPath([string] $Name) {
    return [bool](Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue)
}

function Test-MingwDir([string] $Dir) {
    if ([string]::IsNullOrWhiteSpace($Dir)) { return $false }
    return (Test-Path -LiteralPath (Join-Path $Dir 'bin\gcc.exe') -PathType Leaf)
}

function Get-RustHost {
    $output = & rustc -vV 2>$null
    if ($LASTEXITCODE -ne 0) { return ([string]::Empty) }
    foreach ($line in @($output)) {
        if ($line -like 'host:*') { return $line.Substring(5).Trim() }
    }
    return ([string]::Empty)
}

function Find-Mingw {
    if (-not [string]::IsNullOrWhiteSpace($env:NIMBY_MINGW)) {
        if (Test-MingwDir $env:NIMBY_MINGW) { return $env:NIMBY_MINGW }
        throw "NIMBY_MINGW is '$($env:NIMBY_MINGW)' but '$($env:NIMBY_MINGW)\bin\gcc.exe' does not exist."
    }
    $candidates = @(
        'C:\msys64\mingw64',
        'C:\msys32\mingw32',
        "$env:SystemDrive\mingw64",
        "$env:USERPROFILE\scoop\apps\mingw\current",
        'C:\ProgramData\chocolatey\lib\mingw\tools\install\mingw64',
        'D:\dev\tools\mingw64'
    )
    foreach ($candidate in $candidates) {
        if (Test-MingwDir $candidate) { return $candidate }
    }
    return ([string]::Empty)
}

# Keep build scratch off a full system drive when asked to.
if (-not [string]::IsNullOrWhiteSpace($env:NIMBY_TMP)) {
    New-Item -ItemType Directory -Force -Path $env:NIMBY_TMP | Out-Null
    $env:TEMP = $env:NIMBY_TMP
    $env:TMP = $env:NIMBY_TMP
}

$rustHost = Get-RustHost
if ($rustHost -eq '') {
    throw 'rustc was not found on PATH. Install Rust from https://rustup.rs and try again.'
}

if ($rustHost -like '*-windows-gnu') {
    # The GNU host needs a C toolchain for the C and assembly parts of ring.
    if (-not ((Test-ToolOnPath 'gcc') -and (Test-ToolOnPath 'ar'))) {
        $mingw = Find-Mingw
        if ($mingw -eq '') {
            throw "The Rust host is $rustHost, which needs MinGW-w64, but gcc and ar are not on PATH. Install MinGW-w64 and point NIMBY_MINGW at it, or switch to the MSVC toolchain (rustup default stable-x86_64-pc-windows-msvc) with Visual Studio Build Tools installed."
        }
        $mingwBin = Join-Path $mingw 'bin'
        $env:PATH = "$mingwBin;$env:PATH"
        $extraBin = Join-Path $mingw 'x86_64-w64-mingw32\bin'
        if (Test-Path -LiteralPath $extraBin -PathType Container) {
            $env:PATH = "$extraBin;$env:PATH"
        }
        $env:CC = Join-Path $mingwBin 'gcc.exe'
        $env:AR = Join-Path $mingwBin 'ar.exe'
        $gxx = Join-Path $mingwBin 'g++.exe'
        if (Test-Path -LiteralPath $gxx -PathType Leaf) { $env:CXX = $gxx }
        Write-Host "nimby build: MinGW-w64 from $mingw"
    }
}
elseif ($rustHost -like '*-windows-msvc') {
    Write-Host "nimby build: MSVC host $rustHost"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    if ($args.Count -eq 0) {
        & cargo build
    }
    else {
        & cargo @args
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}
