<#
Removes GCloud Dot from Windows.

  .\uninstall.ps1            remove the app, keep measured session lengths
  .\uninstall.ps1 -Purge     remove everything, including the measurements

Only needed if you installed with install.ps1. The setup.exe installer has its
own entry in Settings > Apps.
#>
[CmdletBinding()]
param([switch]$Purge)

$ErrorActionPreference = 'SilentlyContinue'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\GCloud Dot'
$state = Join-Path $env:LOCALAPPDATA 'GCloudDot'

function Say($text) { Write-Host "  $text" }

# Windows will not delete a running image, and the error says nothing useful.
Get-Process -Name 'gcloud-dot-tray' | Stop-Process -Force
Start-Sleep -Milliseconds 400

# The tray this app replaced, in case it is somehow still registered.
schtasks /End /TN 'GcloudAuthTray' 2>$null | Out-Null
schtasks /Delete /TN 'GcloudAuthTray' /F 2>$null | Out-Null

Remove-Item (Join-Path ([Environment]::GetFolderPath('Startup')) 'GCloud Dot.lnk') -Force
Remove-Item $dest -Recurse -Force

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -like "*$dest*") {
    $cleaned = ($userPath -split ';' | Where-Object { $_ -and $_ -ne $dest }) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $cleaned, 'User')
    Say 'removed from your PATH'
}

Say 'Removed GCloud Dot.'

if ($Purge) {
    Remove-Item $state -Recurse -Force
    Say "Removed measured session lengths in $state."
} else {
    # Each of these took a real session's worth of wall-clock time to observe.
    Say "Kept measured session lengths in $state (-Purge removes them)."
}
