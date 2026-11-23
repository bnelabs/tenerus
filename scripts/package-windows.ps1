param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,
    [Parameter(Mandatory = $true)]
    [string]$Arch,
    [string]$OutDir = "dist"
)

$ErrorActionPreference = "Stop"
$AppRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$CargoToml = Get-Content (Join-Path $AppRoot "Cargo.toml") -Raw
if ($CargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    throw "Could not read version from Cargo.toml"
}
$Version = $Matches[1]

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$StageParent = Join-Path ([System.IO.Path]::GetTempPath()) ("tenerus-win-" + [System.Guid]::NewGuid().ToString("N"))
$BundleName = "Tenerus-$Version-windows-$Arch"
$StageDir = Join-Path $StageParent $BundleName
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

try {
    Copy-Item $BinaryPath (Join-Path $StageDir "tenerus.exe")
    Copy-Item (Join-Path $AppRoot "assets\tenerus.ico") (Join-Path $StageDir "tenerus.ico")
    Copy-Item (Join-Path $AppRoot "README.md") (Join-Path $StageDir "README.md")
    Copy-Item (Join-Path $AppRoot "CREDITS.md") (Join-Path $StageDir "CREDITS.md")
    Copy-Item (Join-Path $AppRoot "CHANGELOG.md") (Join-Path $StageDir "CHANGELOG.md")
    Copy-Item (Join-Path $AppRoot "LICENSE") (Join-Path $StageDir "LICENSE")

    $ZipPath = Join-Path (Resolve-Path $OutDir) "$BundleName.zip"
    if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
    Compress-Archive -Path $StageDir -DestinationPath $ZipPath -CompressionLevel Optimal
    Write-Host "Packaged $ZipPath"
} finally {
    Remove-Item $StageParent -Recurse -Force -ErrorAction SilentlyContinue
}
