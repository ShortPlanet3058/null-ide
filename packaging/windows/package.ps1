# Builds Null's Windows release in target\release:
#  - null-<version>-windows-x86_64-setup.exe, the installer (null.iss: Start menu, `null` on
#    the PATH, "Open with Null"), when Inno Setup 6 is here;
#  - null-<version>-windows-x86_64.zip: null.exe (its icon and version built in) and the
#    licence. Unzipped anywhere, it runs; "Install Shell Command" in Null adds `null`.
#
#     powershell -File packaging\windows\package.ps1
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$name = "null-$version-windows-x86_64"
cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$stage = "target\release\$name"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory $stage | Out-Null
Copy-Item target\release\null.exe, README.md, LICENSE $stage
Compress-Archive -Path $stage -DestinationPath "target\release\$name.zip" -Force
Remove-Item -Recurse -Force $stage
Write-Output "Built target\release\$name.zip"

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
if (Test-Path $iscc) {
    & $iscc /Q "/DVersion=$version" packaging\windows\null.iss
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    Write-Output "Built target\release\$name-setup.exe"
} else {
    Write-Output "(No Inno Setup here: no installer.)"
}
