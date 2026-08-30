[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'TimemAi\bin'),
    [string]$ResourceDir = (Join-Path $env:LOCALAPPDATA 'TimemAi\share\timem\resources'),
    [switch]$KeepPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
foreach ($name in @('timem.exe','timem-web.cmd','timem-native-rs.exe','timem-web.exe','timem-shell.exe')) {
    Remove-Item -LiteralPath (Join-Path $InstallDir $name) -Force -ErrorAction SilentlyContinue
}
Remove-Item -LiteralPath (Join-Path $ResourceDir 'reminder_tips.json') -Force -ErrorAction SilentlyContinue
foreach ($directory in @($ResourceDir, (Split-Path $ResourceDir -Parent), $InstallDir)) {
    if ((Test-Path $directory) -and -not (Get-ChildItem -LiteralPath $directory -Force | Select-Object -First 1)) {
        Remove-Item -LiteralPath $directory -Force
    }
}
if (-not $KeepPath) {
    $canonical = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($current -split ';' | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_) -and [IO.Path]::GetFullPath($_).TrimEnd('\') -ine $canonical
    })
    [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
}
Write-Host "Uninstalled Timem binaries from $InstallDir."
Write-Host 'MEM workspaces, sessions, API credentials, and user configuration were not removed.'
Write-Host 'Rust and Visual C++ Build Tools were not removed.'
