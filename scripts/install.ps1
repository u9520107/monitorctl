param(
    [string]$InstallDirectory = (Join-Path $env:USERPROFILE 'tools\monitorctl'),
    [switch]$NoStart,
    [switch]$NoStartup
)

$dist = Join-Path $PSScriptRoot '..\dist'
$files = 'monitorctl.exe', 'monitorctl-tray.exe', 'README.md', 'LICENSE'
$build = Get-ChildItem -Path $dist -Directory -Filter 'monitorctl-*-windows-x64' |
    ForEach-Object {
        $directory = $_.FullName
        $missing = $files | Where-Object { -not (Test-Path (Join-Path $directory $_)) }
        if (-not $missing) {
            [pscustomobject]@{
                Path = $directory
                BuildTime = (Get-Item (Join-Path $directory 'monitorctl.exe')).LastWriteTimeUtc
            }
        }
    } |
    Sort-Object BuildTime -Descending |
    Select-Object -First 1
if (-not $build) { throw "No portable build found in $dist" }

$trayPath = Join-Path $installDirectory 'monitorctl-tray.exe'
$runningTray = @(Get-Process -Name monitorctl-tray -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -ieq $trayPath })

New-Item -ItemType Directory -Force -Path $installDirectory | Out-Null
if ($runningTray) {
    $runningTray | Stop-Process -ErrorAction Stop
    foreach ($process in $runningTray) {
        Wait-Process -Id $process.Id -Timeout 10 -ErrorAction SilentlyContinue
        if (Get-Process -Id $process.Id -ErrorAction SilentlyContinue) {
            throw "Could not stop $trayPath"
        }
    }
}
Copy-Item ($files | ForEach-Object { Join-Path $build.Path $_ }) -Destination $installDirectory -Force -ErrorAction Stop

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$pathItems = @($userPath -split ';' | Where-Object { $_ })
if (-not ($pathItems | Where-Object { $_.TrimEnd('\') -ieq $installDirectory })) {
    [Environment]::SetEnvironmentVariable('Path', ($pathItems + $installDirectory) -join ';', 'User')
}
$env:Path = "$env:Path;$installDirectory"

$shortcut = Join-Path ([Environment]::GetFolderPath('Startup')) 'Monitorctl.lnk'
if (-not $NoStartup -and -not (Test-Path $shortcut)) {
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($shortcut)
    $link.TargetPath = Join-Path $installDirectory 'monitorctl-tray.exe'
    $link.WorkingDirectory = $installDirectory
    $link.IconLocation = "$installDirectory\monitorctl-tray.exe,0"
    $link.Save()
}

if (-not $NoStart) {
    Start-Process -FilePath $trayPath -WorkingDirectory $installDirectory -ErrorAction Stop
}

Write-Host "Installed Monitorctl to $installDirectory"
