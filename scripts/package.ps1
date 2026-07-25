$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$metadata = & cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE) { throw 'Could not read Cargo package metadata' }
$package = $metadata.packages | Where-Object { $_.name -eq 'monitorctl-core' } | Select-Object -First 1
if (-not $package) { throw 'Could not find monitorctl-core package metadata' }

Push-Location $root
try {
    & cargo build --release --bins
    if ($LASTEXITCODE) { throw 'Release build failed' }

    $stage = Join-Path $root "dist\monitorctl-$($package.version)-windows-x64"
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item @(
        (Join-Path $metadata.target_directory 'release\monitorctl.exe'),
        (Join-Path $metadata.target_directory 'release\monitorctl-tray.exe'),
        (Join-Path $root 'README.md'),
        (Join-Path $root 'LICENSE')
    ) -Destination $stage -Force
    Write-Host "Packaged Monitorctl to $stage"
}
finally {
    Pop-Location
}
