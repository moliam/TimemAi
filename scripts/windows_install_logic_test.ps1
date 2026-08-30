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

    $installText = [IO.File]::ReadAllText((Join-Path $root 'install.ps1'))
    $uninstallText = [IO.File]::ReadAllText((Join-Path $root 'uninstall.ps1'))
    foreach ($required in @(
        "Invoke-Cargo @('fetch', '--locked')",
        "Invoke-Cargo @('build', '--locked', '-p', 'timem_shell', '-p', 'timem_web', '--release')",
        'Copy-FileAtomically',
        "SetEnvironmentVariable('Path'",
        'x86_64-pc-windows-msvc'
    )) {
        if (-not $installText.Contains($required)) { throw "install.ps1 missing contract: $required" }
    }
    foreach ($protected in @('MEM workspaces', 'API credentials', 'user configuration')) {
        if (-not $uninstallText.Contains($protected)) { throw "uninstall.ps1 missing preservation notice: $protected" }
    }
    Write-Output 'windows_install_logic_test: ok'
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
