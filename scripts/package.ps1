# Packages Nimby as a portable Windows application.
#
#   powershell -File scripts\package.ps1 [-Runtime win-x64] [-Output dist]
#                                        [-SingleFile] [-SkipSmoke] [-NoZip]
#
# Produces:
#   dist\nimby-<version>-<runtime>\          portable folder (zip = drop-in)
#   dist\nimby-<version>-<runtime>.zip
#   dist\SHA256SUMS.txt
#
# The folder is self-contained: Nimby.exe (GUI), bin\nimby.exe (CLI, used by the
# GUI from the same folder), data\ (portable config, created on first run), the
# brand assets, the licence/notice files and the docs/tutorial.
param(
    [string]$Version = '',
    [string]$Runtime = 'win-x64',
    [string]$Output = 'dist',
    [switch]$SingleFile,
    [switch]$SkipSmoke,
    [switch]$NoZip
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    if ([string]::IsNullOrWhiteSpace($Version)) {
        $match = Select-String -Path 'Cargo.toml' -Pattern '^version = "(.+)"' | Select-Object -First 1
        if (-not $match) { throw 'cannot read the workspace version from Cargo.toml' }
        $Version = $match.Matches[0].Groups[1].Value
    }

    $name = "nimby-$Version-$Runtime"
    $stage = Join-Path $Output $name
    Write-Host "== packaging $name =="
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null

    Write-Host '== building CLI (release) =='
    & (Join-Path $PSScriptRoot 'build.ps1') 'build' '--release'
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    # The CLI lives in bin\ because Windows would treat a "Nimby.exe" next to a
    # "nimby.exe" as the same file. A CLI started from bin\ shares the data\
    # folder of its parent, so both binaries use one portable configuration.
    New-Item -ItemType Directory -Force (Join-Path $stage 'bin') | Out-Null
    Copy-Item 'target\release\nimby.exe' (Join-Path $stage 'bin\nimby.exe')

    Write-Host '== publishing GUI =='
    $publishArgs = @(
        'publish', 'gui\Nimby.App\Nimby.App.csproj',
        '-c', 'Release',
        '-r', $Runtime,
        '--self-contained', 'true',
        "-p:Version=$Version",
        "-p:PublishSingleFile=$(if ($SingleFile) { 'true' } else { 'false' })",
        '-o', $stage
    )
    if ($SingleFile) {
        $publishArgs += @(
            '-p:IncludeNativeLibrariesForSelfExtract=true',
            '-p:EnableCompressionInSingleFile=true'
        )
    }
    dotnet @publishArgs
    if ($LASTEXITCODE -ne 0) { throw 'dotnet publish failed' }

    Write-Host '== staging assets and docs =='
    New-Item -ItemType Directory -Force (Join-Path $stage 'assets\branding') | Out-Null
    Copy-Item 'assets\branding\*' (Join-Path $stage 'assets\branding') -Recurse -Force

    # A writable data\ folder next to the exe is what makes the app portable:
    # config.json and the hash cache live there instead of %APPDATA%.
    New-Item -ItemType Directory -Force (Join-Path $stage 'data') | Out-Null
    Set-Content -Path (Join-Path $stage 'data\README.txt') -Encoding utf8 -Value @'
Nimby keeps its configuration and hash cache in this folder while it stays
writable, which makes this installation portable: move or copy the whole folder
and your repositories, paths and settings come with it.

Delete config.json to start from scratch. If this folder is read-only (for
example under Program Files), Nimby falls back to %APPDATA%\Nimby automatically.
'@

    foreach ($file in 'LICENSE', 'NOTICE', 'THIRD-PARTY-NOTICES.md', 'CHANGELOG.md', 'README.md') {
        if (Test-Path $file) { Copy-Item $file $stage }
    }
    if (Test-Path 'LICENSES') {
        Copy-Item 'LICENSES' $stage -Recurse -Force
    }
    if (Test-Path 'docs') {
        New-Item -ItemType Directory -Force (Join-Path $stage 'docs') | Out-Null
        Copy-Item 'docs\*' (Join-Path $stage 'docs') -Recurse -Force
    }

    if (-not $SkipSmoke) {
        Write-Host '== running the packaged GUI self-check =='
        $probe = Join-Path ([System.IO.Path]::GetTempPath()) ("nimby-package-" + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force $probe | Out-Null
        try {
            # A throwaway config keeps the staged folder clean and proves the
            # binary works from an arbitrary working directory.
            $env:NIMBY_CONFIG = Join-Path $probe 'config.json'
            $env:NIMBY_SMOKE = '1'
            $exe = Join-Path $stage 'Nimby.exe'
            & $exe
            if ($LASTEXITCODE -ne 0) { throw "packaged GUI smoke test failed (exit $LASTEXITCODE)" }
        } finally {
            Remove-Item Env:NIMBY_CONFIG -ErrorAction SilentlyContinue
            Remove-Item Env:NIMBY_SMOKE -ErrorAction SilentlyContinue
            Remove-Item -Recurse -Force $probe -ErrorAction SilentlyContinue
        }
    }

    # Nothing generated during the smoke test may end up in the zip.
    Remove-Item (Join-Path $stage 'data\config.json') -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $stage 'data\hashcache.json') -ErrorAction SilentlyContinue

    # The smoke run leaves the CLI it drove to finish shutting down; a file that is
    # still open would make the zip fail with a sharing violation.
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline) {
        $busy = @(Get-Process -Name 'Nimby', 'nimby' -ErrorAction SilentlyContinue | Where-Object {
                try { $_.Path -and $_.Path.StartsWith($stage, [System.StringComparison]::OrdinalIgnoreCase) }
                catch { $false }
            })
        if ($busy.Count -eq 0) { break }
        Start-Sleep -Milliseconds 250
    }

    if (-not $NoZip) {
        Write-Host '== zipping =='
        $zip = Join-Path $Output "$name.zip"
        if (Test-Path $zip) { Remove-Item -Force $zip }
        # A virus scanner can hold a freshly published file for a moment, which
        # shows up as an UnauthorizedAccessException inside Compress-Archive.
        $attempt = 0
        while ($true) {
            $attempt++
            try {
                Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal -ErrorAction Stop
                break
            } catch {
                if ($attempt -ge 6) { throw }
                Write-Host "   zip retry $attempt : $($_.Exception.Message)"
                Start-Sleep -Seconds (3 * $attempt)
                if (Test-Path $zip) { Remove-Item -Force $zip -ErrorAction SilentlyContinue }
            }
        }

        $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
        $sums = Join-Path $Output 'SHA256SUMS.txt'
        "$hash  $name.zip" | Set-Content -Path $sums -Encoding utf8
        Write-Host "packaged $zip"
        Write-Host "sha256   $hash"
    } else {
        Write-Host "staged   $stage"
    }
} finally {
    Pop-Location
}
