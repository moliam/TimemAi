[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'TimemAi\bin'),
    [string]$ResourceDir = (Join-Path $env:LOCALAPPDATA 'TimemAi\share\timem\resources'),
    [switch]$SkipPathUpdate
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$RootDir = $PSScriptRoot
$MinRustVersion = [Version]'1.78.0'

function Get-CommandPath([string]$Name) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -eq $command) { return $null }
    $command.Source
}

function Assert-WindowsPrerequisites {
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'Timem requires 64-bit Windows.' }
    $script:CargoPath = Get-CommandPath 'cargo.exe'
    $rustc = Get-CommandPath 'rustc.exe'
    if (-not $script:CargoPath -or -not $rustc) {
        throw 'Install stable Rust for x86_64-pc-windows-msvc from https://rustup.rs/, reopen PowerShell, and rerun .\install.ps1.'
    }
    $versionText = (& $rustc --version).Split(' ', [StringSplitOptions]::RemoveEmptyEntries)[1].Split('-')[0]
    if ([Version]$versionText -lt $MinRustVersion) {
        throw "Rust $MinRustVersion or newer is required. Run 'rustup update stable'."
    }
    $hostTriple = (& $rustc -vV | Select-String '^host:').Line.Split(':', 2)[1].Trim()
    if ($hostTriple -ne 'x86_64-pc-windows-msvc') {
        throw "The x86_64-pc-windows-msvc Rust host is required; found $hostTriple."
    }

    $script:VsDevCmdPath = $null
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path $vswhere) {
        $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1
        if (-not [string]::IsNullOrWhiteSpace($installation)) {
            $candidate = Join-Path $installation 'Common7\Tools\VsDevCmd.bat'
            if (Test-Path $candidate) { $script:VsDevCmdPath = $candidate }
        }
    }
    if (-not $script:VsDevCmdPath -and -not (Get-CommandPath 'link.exe')) {
        throw 'Microsoft Visual C++ x64 Build Tools are required. Install Desktop development with C++.'
    }
}

function Invoke-Cargo([string[]]$Arguments) {
    if ($script:VsDevCmdPath) {
        $quotedCargo = '"' + $script:CargoPath + '"'
        $argumentLine = ($Arguments | ForEach-Object {
            if ($_ -match '[\s"]') { '"' + $_.Replace('"', '\"') + '"' } else { $_ }
        }) -join ' '
        $commandLine = 'call "' + $script:VsDevCmdPath + '" -arch=x64 -host_arch=x64 >nul && ' + $quotedCargo + ' ' + $argumentLine
        & cmd.exe /d /s /c $commandLine
    } else {
        & $script:CargoPath @Arguments
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo command failed with exit code $LASTEXITCODE" }
}

function Copy-FileAtomically([string]$Source, [string]$Destination) {
    $directory = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $temporary = Join-Path $directory ('.' + [IO.Path]::GetFileName($Destination) + '.tmp.' + [Guid]::NewGuid().ToString('N'))
    try {
        Copy-Item -LiteralPath $Source -Destination $temporary -Force
        Move-Item -LiteralPath $temporary -Destination $Destination -Force
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Write-TextAtomically([string]$Destination, [string]$Content) {
    $directory = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $temporary = Join-Path $directory ('.' + [IO.Path]::GetFileName($Destination) + '.tmp.' + [Guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::WriteAllText($temporary, $Content, [Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $Destination -Force
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Install-CommandArtifacts([string]$UnifiedExecutable, [string]$Directory) {
    Copy-FileAtomically $UnifiedExecutable (Join-Path $Directory 'timem.exe')

    # Remove independently executable legacy commands before installing the
    # forwarding shim. On Windows, an old .exe would take precedence over .cmd.
    foreach ($legacyName in @('timem-web.exe', 'timem-native-rs.exe', 'timem-shell.exe')) {
        Remove-Item -LiteralPath (Join-Path $Directory $legacyName) -Force -ErrorAction SilentlyContinue
    }

    $webShim = "@echo off`r`n`"%~dp0timem.exe`" %*`r`nexit /b %ERRORLEVEL%`r`n"
    Write-TextAtomically (Join-Path $Directory 'timem-web.cmd') $webShim
}

function Add-UserPathEntry([string]$Directory) {
    $canonical = [IO.Path]::GetFullPath($Directory).TrimEnd('\')
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($current -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($entries | Where-Object { [IO.Path]::GetFullPath($_).TrimEnd('\') -ieq $canonical }) { return $false }
    [Environment]::SetEnvironmentVariable('Path', ((@($entries) + $canonical) -join ';'), 'User')
    if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $canonical })) {
        $env:Path = $env:Path.TrimEnd(';') + ';' + $canonical
    }
    $true
}

function Invoke-TimemInstall {
    Assert-WindowsPrerequisites
    if (-not (Test-Path (Join-Path $RootDir 'interfaces\web\dist\index.html'))) {
        throw 'Embedded Timem Web assets are missing from this source package.'
    }
    Push-Location $RootDir
    try {
        Write-Host 'Fetching locked Rust dependencies...'
        Invoke-Cargo @('fetch', '--locked')
        Write-Host 'Building the unified Timem CLI for Windows...'
        Invoke-Cargo @('build', '--locked', '-p', 'timem_web', '--release')
    } finally { Pop-Location }

    $timem = Join-Path $RootDir 'target\release\timem.exe'
    $tips = Join-Path $RootDir 'resources\reminder_tips.json'
    foreach ($path in @($timem, $tips)) { if (-not (Test-Path $path)) { throw "Missing output: $path" } }
    Install-CommandArtifacts $timem $InstallDir
    Copy-FileAtomically $tips (Join-Path $ResourceDir 'reminder_tips.json')
    $pathAdded = -not $SkipPathUpdate -and (Add-UserPathEntry $InstallDir)

    Write-Host ''
    Write-Host 'TimemAi installation complete.'
    Write-Host "  Timem CLI:           $(Join-Path $InstallDir 'timem.exe')"
    Write-Host "  Compatibility shim: $(Join-Path $InstallDir 'timem-web.cmd')"
    Write-Host "  Resources:               $ResourceDir"
    if ($pathAdded) { Write-Host 'The user PATH was updated. Open a new terminal before invoking Timem by name.' }
    Write-Host 'Start Timem Web (default): timem'
    Write-Host 'Start the terminal Shell: timem --shell'
    Write-Host 'No environment file is required to open the local Web UI.'
}

if ($MyInvocation.InvocationName -ne '.') { Invoke-TimemInstall }
