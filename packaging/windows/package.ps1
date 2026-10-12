# Builds Null's Windows release: target\release\null-<version>-windows-x86_64.zip, holding
# null.exe (its icon and version built in) and the licences. Unzipped anywhere, it runs;
# "Install Shell Command" in Null adds `null` for terminals.
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
Copy-Item target\release\null.exe, README.md, LICENSE-MIT, LICENSE-APACHE $stage
Compress-Archive -Path $stage -DestinationPath "target\release\$name.zip" -Force
Remove-Item -Recurse -Force $stage
Write-Output "Built target\release\$name.zip"
