# One-command online upgrade for TimemAi on Windows.
# Downloads the prebuilt release archive for this platform, verifies its
# checksum, stops the running Timem process, and replaces the installed
# timem.exe in place. MEM workspaces, Sessions, credentials, and user
# configuration are never touched.
#
# Usage:
#   irm https://raw.githubusercontent.com/moliam/TimemAi/main/upgrade-online.ps1 | iex
# Pin a version:
#   $env:TIMEM_VERSION = 'v2.5.0'; irm .../upgrade-online.ps1 | iex
[CmdletBinding()]
param(
    [string]$Version = $(if ($env:TIMEM_VERSION) { $env:TIMEM_VERSION } else { 'latest' }),
    [string]$Repository = $(if ($env:TIMEM_INSTALL_REPOSITORY) { $env:TIMEM_INSTALL_REPOSITORY } else { 'moliam/TimemAi' })
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-UpgradeIdentifier([string]$Value, [string]$Label) {
    if ([string]::IsNullOrWhiteSpace($Value) -or $Value -notmatch '^[A-Za-z0-9._-]+$') {
        throw "Invalid ${Label}: $Value"
    }
}

function Resolve-UpgradeVersion([string]$Requested, [string]$Repo) {
    if ($Requested -ne 'latest') { return $Requested }
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Method Head -Uri "https://github.com/$Repo/releases/latest"
        $base = $response.BaseResponse
        $uriProp = $base.PSObject.Properties['ResponseUri']
        $msgProp = $base.PSObject.Properties['RequestMessage']
        if ($null -ne $uriProp -and $null -ne $uriProp.Value) {
            return ([Uri]$uriProp.Value.AbsoluteUri).AbsolutePath.TrimEnd('/').Split('/')[-1]
        }
        if ($null -ne $msgProp -and $null -ne $msgProp.Value -and $null -ne $msgProp.Value.RequestUri) {
            return ([Uri]$msgProp.Value.RequestUri.AbsoluteUri).AbsolutePath.TrimEnd('/').Split('/')[-1]
        }
        throw 'The HTTP client did not expose the final redirect URI.'
    } catch {
        throw "Failed to resolve the latest TimemAi release: $($_.Exception.Message)"
    }
}

function Get-TimemInstallCandidates {
    $candidates = @()
    $defaultBin = Join-Path $env:LOCALAPPDATA 'TimemAi\bin'
    if (Test-Path (Join-Path $defaultBin 'timem.exe')) { $candidates += $defaultBin }
    try { $where = @(where.exe timem 2>$null) } catch { $where = @() }
    foreach ($hit in $where) {
        $dir = Split-Path -Parent $hit
        if ($candidates -notcontains $dir) { $candidates += $dir }
    }
    if ($env:TIMEM_UPGRADE_BIN_DIR) { $candidates = @($env:TIMEM_UPGRADE_BIN_DIR) }
    $candidates
}

function Stop-TimemProcesses {
    Stop-Process -Name timem,timem-web -ErrorAction SilentlyContinue
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        if (@(Get-Process -Name timem,timem-web -ErrorAction SilentlyContinue).Count -eq 0) { return }
        Start-Sleep -Milliseconds 500
    }
    $alive = @(Get-Process -Name timem,timem-web -ErrorAction SilentlyContinue)
    if ($alive.Count -gt 0) {
        throw ("Timem is still running (pid " + ($alive.Id -join ',') + "); close it and rerun the upgrade.")
    }
}

function Invoke-TimemUpgrade {
    if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
        throw "Invalid GitHub repository: $Repository"
    }
    Assert-UpgradeIdentifier $Version 'release version'

    if ([Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITECTURE') -eq 'ARM64') {
        throw 'Windows on ARM is not covered by prebuilt archives. Upgrade from source: https://github.com/moliam/TimemAi#install-or-update'
    }

    $resolvedVersion = Resolve-UpgradeVersion $Version $Repository
    Assert-UpgradeIdentifier $resolvedVersion 'release version'
    $asset = "timem-$resolvedVersion-x86_64-pc-windows-msvc.zip"
    $base = "https://github.com/$Repository/releases/download/$resolvedVersion"

    $candidates = Get-TimemInstallCandidates
    if ($candidates.Count -eq 0) {
        throw 'timem.exe was not found. Install first: irm https://raw.githubusercontent.com/moliam/TimemAi/main/install-online.ps1 | iex'
    }

    $temp = Join-Path ([IO.Path]::GetTempPath()) ('timem-upgrade-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        Write-Host "Downloading TimemAi $resolvedVersion (x86_64-pc-windows-msvc)..."
        $archive = Join-Path $temp $asset
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $archive
        $sidecar = Join-Path $temp "$asset.sha256"
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256" -OutFile $sidecar

        $expected = ([IO.File]::ReadAllText($sidecar) -split '\s+')[0]
        if (-not $expected -or $expected -notmatch '^[A-Fa-f0-9]{64}$') {
            throw "Malformed checksum sidecar for $asset"
        }
        $actual = (Get-FileHash $archive -Algorithm SHA256).Hash
        if ($actual -ne $expected) {
            throw "Checksum mismatch for $asset (expected $expected, got $actual)"
        }
        Write-Host 'Checksum verified.'

        Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $temp 'extract') -Force
        $staged = Join-Path $temp 'extract\timem.exe'
        if (-not (Test-Path $staged)) { throw "Archive does not contain timem.exe" }

        Write-Host 'Stopping Timem...'
        Stop-TimemProcesses

        $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
        foreach ($bin in $candidates) {
            $exe = Join-Path $bin 'timem.exe'
            if (-not (Test-Path $exe)) { continue }
            $backup = Join-Path $bin "timem.exe.bak.$stamp"
            Copy-Item $exe $backup -Force
            $pending = Join-Path $bin ".timem.upgrade.$stamp"
            Copy-Item $staged $pending -Force
            try {
                Move-Item $pending $exe -Force
            } catch {
                Remove-Item $pending -Force -ErrorAction SilentlyContinue
                throw "Failed to replace $exe (is Timem still running?): $($_.Exception.Message)"
            }
            $shim = "@echo off`r`n`"%~dp0timem.exe`" %*`r`nexit /b %ERRORLEVEL%`r`n"
            [IO.File]::WriteAllText((Join-Path $bin 'timem-web.cmd'), $shim)

            & $exe --help *> $null
            if ($LASTEXITCODE -ne 0) {
                Copy-Item $backup $exe -Force
                throw "Post-install check failed for $exe; rolled back to the previous version."
            }
            Write-Host "TimemAi upgraded to $resolvedVersion."
            Write-Host "  binary:  $exe"
            Write-Host "  backup:  $backup"
        }
        Write-Host 'User data (MEM workspaces, Sessions, credentials, configuration) is untouched.'
        Write-Host 'Open a NEW terminal and start Timem as usual.'
    } finally {
        Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-TimemUpgrade }
