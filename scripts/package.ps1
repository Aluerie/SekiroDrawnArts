<#
.SYNOPSIS
Builds the release DLL and puts everything a player needs into one folder, plus a zip of it.

.DESCRIPTION
Creates, relative to the repository root:
  dist\SekiroDrawnArts\                  drawn_arts.me3, drawn_arts\ (DLL and settings), README.md
  dist\SekiroDrawnArts-v<version>.zip    that folder, zipped, ready to upload to a release
The version comes from Cargo.toml. Any previous dist folder is replaced.
The wheel images in drawn_arts\icons\ are left out: they are the game's own art, so players
extract them from their copy of the game.
Works in Windows PowerShell 5.1 and PowerShell 7.
#>
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$version = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages[0].version
$folder = 'dist\SekiroDrawnArts'
$zip = "dist\SekiroDrawnArts-v$version.zip"

if (Test-Path dist) { Remove-Item dist -Recurse -Force }
New-Item -ItemType Directory "$folder\drawn_arts" | Out-Null
Copy-Item drawn_arts.me3, README.md $folder
Copy-Item target\release\drawn_arts.dll, drawn_arts\drawn_arts.ini "$folder\drawn_arts"
if (Test-Path LICENSE) { Copy-Item LICENSE $folder }
Compress-Archive -Path $folder -DestinationPath $zip

Write-Host "Folder: $folder"
Write-Host "Zip:    $zip"
