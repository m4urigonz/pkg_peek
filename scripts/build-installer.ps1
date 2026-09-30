<#
.SYNOPSIS
    Compila PKG Peek en release y genera dist\PkgPeek-Setup-<versión>.exe.

.DESCRIPTION
    La versión sale de [workspace.package] en Cargo.toml. Necesita Inno Setup 6
    (winget install JRSoftware.InnoSetup).
#>
param(
    [switch]$NoBuild,
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

if (-not $Version) {
    $cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    if ($cargo -notmatch '(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
        throw 'No se encontró la versión en [workspace.package] de Cargo.toml.'
    }
    $Version = $Matches[1]
}

$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'No se encontró ISCC.exe. Instala Inno Setup 6: winget install JRSoftware.InnoSetup' }

Push-Location $root
try {
    if (-not $NoBuild) {
        Write-Host "Compilando PKG Peek $Version (release)..." -ForegroundColor Cyan
        cargo build --release -p pkgpeek -p pkgpeek-shell
        if ($LASTEXITCODE -ne 0) { throw 'La compilación falló.' }
    }
    Write-Host 'Generando el instalador...' -ForegroundColor Cyan
    & $iscc /Q "/DAppVersion=$Version" (Join-Path $root 'installer\PkgPeek.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC falló (código $LASTEXITCODE)." }
    $setup = Join-Path $root "dist\PkgPeek-Setup-$Version.exe"
    Write-Host "Listo: $setup" -ForegroundColor Green
} finally {
    Pop-Location
}
