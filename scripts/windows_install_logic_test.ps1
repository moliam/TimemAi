$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
. (Join-Path $root 'install.ps1') -SkipPathUpdate

$temp = Join-Path ([IO.Path]::GetTempPath()) ('timem-install-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    $source = Join-Path $temp 'source.bin'
    $destination = Join-Path $temp 'nested\destination.bin'
    [IO.File]::WriteAllText($source, 'new binary')
    Copy-FileAtomically $source $destination
    if ([IO.File]::ReadAllText($destination) -ne 'new binary') { throw 'atomic copy content mismatch' }
    if (Get-ChildItem (Split-Path $destination -Parent) -Filter '*.tmp.*') { throw 'atomic copy left temporary files' }

    $shim = Join-Path $temp 'nested\timem-web.cmd'
    Write-TextAtomically $shim "@echo off`r`n`\"%~dp0timem.exe`\" %*`r`nexit /b %ERRORLEVEL%`r`n"
    $shimText = [IO.File]::ReadAllText($shim)
    if (-not $shimText.Contains('"%~dp0timem.exe" %*')) { throw 'compatibility shim must forward every argument to timem.exe' }

    $legacyDir = Join-Path $temp 'legacy-bin'
    New-Item -ItemType Directory -Path $legacyDir | Out-Null
    $unified = Join-Path $temp 'timem.exe'
    [IO.File]::WriteAllText($unified, 'unified executable')
    foreach ($legacyName in @('timem-web.exe', 'timem-native-rs.exe', 'timem-shell.exe')) {
        [IO.File]::WriteAllText((Join-Path $legacyDir $legacyName), 'legacy executable')
    }
    Install-CommandArtifacts $unified $legacyDir
    if ([IO.File]::ReadAllText((Join-Path $legacyDir 'timem.exe')) -ne 'unified executable') {
        throw 'upgrade did not install the unified executable'
    }
    foreach ($legacyName in @('timem-web.exe', 'timem-native-rs.exe', 'timem-shell.exe')) {
        if (Test-Path (Join-Path $legacyDir $legacyName)) { throw "upgrade retained legacy executable: $legacyName" }
    }
    if (-not (Test-Path (Join-Path $legacyDir 'timem-web.cmd'))) { throw 'upgrade did not install compatibility shim' }

    $installText = [IO.File]::ReadAllText((Join-Path $root 'install.ps1'))
    $uninstallText = [IO.File]::ReadAllText((Join-Path $root 'uninstall.ps1'))
    foreach ($required in @(
        "Invoke-Cargo @('fetch', '--locked')",
        "Invoke-Cargo @('build', '--locked', '--release', '--bin', 'timem')",
        "target\release\timem.exe",
        'Install-CommandArtifacts $timem $InstallDir',
        "Write-TextAtomically (Join-Path `$Directory 'timem-web.cmd')",
        'Copy-FileAtomically',
        "SetEnvironmentVariable('Path'",
        'x86_64-pc-windows-msvc'
    )) {
        if (-not $installText.Contains($required)) { throw "install.ps1 missing contract: $required" }
    }
    foreach ($forbidden in @("'-p', 'timem_shell'", 'target\release\timem-web.exe')) {
        if ($installText.Contains($forbidden)) { throw "install.ps1 must not install a second executable: $forbidden" }
    }
    foreach ($protected in @('MEM workspaces', 'API credentials', 'user configuration')) {
        if (-not $uninstallText.Contains($protected)) { throw "uninstall.ps1 missing preservation notice: $protected" }
    }
    Write-Output 'windows_install_logic_test: ok'
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
