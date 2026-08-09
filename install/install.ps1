<#
GCloud Dot installer for Windows.

  irm https://raw.githubusercontent.com/nicglazkov/gcloud-dot/main/install/install.ps1 | iex

Installs for the current user only, under %LOCALAPPDATA%\Programs\GCloud Dot.
Nothing here needs administrator rights.
#>
[CmdletBinding()]
param(
    [switch]$NoGui,
    [switch]$NoStartup,
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$repo = 'nicglazkov/gcloud-dot'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\GCloud Dot'

function Say($text) { Write-Host "  $text" }

if (-not $Version) {
    $latest = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" `
        -Headers @{ 'User-Agent' = 'gcloud-dot-installer' }
    $Version = $latest.tag_name
}

Write-Host ""
Write-Host "GCloud Dot $Version  (windows-x86_64)"
Write-Host ""

$asset = 'gcloud-dot-windows-x86_64.zip'
$url = "https://github.com/$repo/releases/download/$Version/$asset"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("gclouddot-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    Say 'downloading...'
    $zip = Join-Path $tmp $asset
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $tmp -Force

    # Windows refuses to overwrite a running image, and the error it gives is
    # a bare access-denied that tells the user nothing.
    Get-Process -Name 'gcloud-dot-tray' -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400

    New-Item -ItemType Directory -Path $dest -Force | Out-Null
    Copy-Item (Join-Path $tmp 'gcloud-dot.exe') $dest -Force
    Say "installed $dest\gcloud-dot.exe"
    if (-not $NoGui) {
        Copy-Item (Join-Path $tmp 'gcloud-dot-tray.exe') $dest -Force
        Say "installed $dest\gcloud-dot-tray.exe"
    }

    # PATH for this user only.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -notlike "*$dest*") {
        $joined = if ([string]::IsNullOrEmpty($userPath)) { $dest } else { "$userPath;$dest" }
        [Environment]::SetEnvironmentVariable('Path', $joined, 'User')
        Say 'added to your PATH (restart your terminal to pick it up)'
    }

    if (-not $NoGui -and -not $NoStartup) {
        $startup = [Environment]::GetFolderPath('Startup')
        $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut(
            (Join-Path $startup 'GCloud Dot.lnk'))
        $shortcut.TargetPath = Join-Path $dest 'gcloud-dot-tray.exe'
        $shortcut.Description = 'GCloud Dot'
        $shortcut.Save()
        Say 'set to start at login'

        Start-Process (Join-Path $dest 'gcloud-dot-tray.exe')
        Say 'started'
    }

    Write-Host ""
    Say "Run 'gcloud-dot' for a one-off check."
    if (-not $NoGui) {
        Write-Host ""
        Say 'Windows 11 hides new tray icons behind the overflow arrow. Drag the'
        Say 'dot out of it once and it stays on the taskbar.'
    }
    Write-Host ""
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
