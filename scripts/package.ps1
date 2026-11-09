# Packages Nimby for distribution.
#
#   powershell -File scripts\package.ps1 [-Runtime win-x64] [-Output dist]
#
# Produces:
#   dist\nimby-<version>-<runtime>\        portable folder
#   dist\nimby-<version>-<runtime>.zip     zip of that folder
param(
    [string]$Runtime = 'win-x64',
    [string]$Output = 'dist'
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Write-Host '== building CLI (release) =='
    & (Join-Path $PSScriptRoot 'build.ps1') 'build' '--release'
    if (-not $?) { throw 'cargo build failed' }

    $version = (Select-String -Path 'crates\nimby-core\Cargo.toml' -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
    $name = "nimby-$version-$Runtime"
    $stage = Join-Path $Output $name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null

    Write-Host '== publishing GUI =='
    dotnet publish 'gui\Nimby.App\Nimby.App.csproj' `
        -c Release `
        -r $Runtime `
        --self-contained true `
        -p:PublishSingleFile=false `
        -o (Join-Path $stage 'gui')
    if (-not $?) { throw 'dotnet publish failed' }

    Copy-Item 'target\release\nimby.exe' (Join-Path $stage 'nimby.exe')
    Copy-Item 'README.md' $stage
    Copy-Item 'docs\FORMAT.md' $stage
    Copy-Item 'target\release\nimby.exe' (Join-Path $stage 'gui\nimby.exe')

    Write-Host '== zipping =='
    $zip = Join-Path $Output "$name.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
    Write-Host "packaged $zip"
} finally {
    Pop-Location
}
