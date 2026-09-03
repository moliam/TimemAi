[CmdletBinding()]
param(
    [string]$Version = $(if ($env:TIMEM_VERSION) { $env:TIMEM_VERSION } else { 'latest' }),
    [string]$Repository = $(if ($env:TIMEM_INSTALL_REPOSITORY) { $env:TIMEM_INSTALL_REPOSITORY } else { 'moliam/TimemAi' }),
    [string]$InstallDir,
    [string]$ResourceDir,
    [switch]$SkipPathUpdate
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-InstallIdentifier([string]$Value, [string]$Label, [string]$Pattern) {
    if ([string]::IsNullOrWhiteSpace($Value) -or $Value -notmatch $Pattern) {
        throw "Invalid ${Label}: $Value"
    }
}

function Resolve-TimemVersion([string]$RequestedVersion, [string]$Repo) {
    if ($RequestedVersion -ne 'latest') {
        Assert-InstallIdentifier $RequestedVersion 'release version' '^[A-Za-z0-9._-]+$'
        return $RequestedVersion
    }

    try {
        $response = Invoke-WebRequest -UseBasicParsing -Method Head -Uri "https://github.com/$Repo/releases/latest"
        $baseResponse = $response.BaseResponse
        $responseUriProperty = $baseResponse.PSObject.Properties['ResponseUri']
        $requestMessageProperty = $baseResponse.PSObject.Properties['RequestMessage']
        if ($null -ne $responseUriProperty -and $null -ne $responseUriProperty.Value) {
            $effective = $responseUriProperty.Value.AbsoluteUri
        } elseif ($null -ne $requestMessageProperty -and
            $null -ne $requestMessageProperty.Value -and
            $null -ne $requestMessageProperty.Value.RequestUri) {
            $effective = $requestMessageProperty.Value.RequestUri.AbsoluteUri
        } else {
            throw 'The HTTP client did not expose the final redirect URI.'
        }
    } catch {
        throw "Failed to resolve the latest TimemAi release: $($_.Exception.Message)"
    }
    $resolved = ([Uri]$effective).AbsolutePath.TrimEnd('/').Split('/')[-1]
    if ($resolved -eq 'latest') { throw 'GitHub did not resolve a latest TimemAi release.' }
    Assert-InstallIdentifier $resolved 'release version' '^[A-Za-z0-9._-]+$'
    $resolved
}

function Invoke-TimemOnlineInstall {
    Assert-InstallIdentifier $Repository 'GitHub repository' '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'Timem requires 64-bit Windows.' }

    $resolvedVersion = Resolve-TimemVersion $Version $Repository
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ('timem-online-install-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporary | Out-Null
    try {
        $archive = Join-Path $temporary 'timem.zip'
        $archiveUrl = "https://github.com/$Repository/archive/refs/tags/$resolvedVersion.zip"
        Write-Host "Downloading TimemAi $resolvedVersion from GitHub..."
        Invoke-WebRequest -UseBasicParsing -Uri $archiveUrl -OutFile $archive
        Expand-Archive -LiteralPath $archive -DestinationPath $temporary -Force

        $sourceCandidates = @(Get-ChildItem -LiteralPath $temporary -Directory | Where-Object {
            (Test-Path (Join-Path $_.FullName 'install.ps1')) -and
            (Test-Path (Join-Path $_.FullName 'Cargo.lock')) -and
            (Test-Path (Join-Path $_.FullName 'interfaces\web\dist\index.html'))
        })
        if ($sourceCandidates.Count -ne 1) {
            throw 'Downloaded TimemAi archive is incomplete or has an unexpected layout.'
        }
        $source = $sourceCandidates[0]

        $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $source.FullName 'install.ps1'))
        if ($InstallDir) { $arguments += @('-InstallDir', $InstallDir) }
        if ($ResourceDir) { $arguments += @('-ResourceDir', $ResourceDir) }
        if ($SkipPathUpdate) { $arguments += '-SkipPathUpdate' }
        Write-Host "Building and installing TimemAi from release $resolvedVersion..."
        $previousSourceKind = $env:TIMEM_INSTALL_SOURCE_KIND
        $env:TIMEM_INSTALL_SOURCE_KIND = 'online'
        try {
            & powershell.exe @arguments
            $installerExitCode = $LASTEXITCODE
        } finally {
            $env:TIMEM_INSTALL_SOURCE_KIND = $previousSourceKind
        }
        if ($installerExitCode -ne 0) { throw "TimemAi installer failed with exit code $installerExitCode" }
    } finally {
        Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-TimemOnlineInstall }
