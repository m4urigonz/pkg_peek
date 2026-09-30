<#
.SYNOPSIS
    Compila e instala PKG Peek (extensión del Explorador para PKG de PS4).

.DESCRIPTION
    1. Compila en release como usuario normal (cargo build).
    2. Se eleva a administrador (UAC) para copiar los archivos a
       "C:\Program Files\PkgPeek" y registrar la DLL con regsvr32.
    Se puede ejecutar de nuevo para actualizar: la DLL anterior, si está en
    uso por el Explorador, se renombra y se borra en la siguiente instalación.

.PARAMETER RestartExplorer
    Reinicia el Explorador al terminar (cierra sus ventanas) para que la DLL
    nueva se cargue y se limpien las cachés de iconos y miniaturas.
#>
param(
    [switch]$RestartExplorer,
    [switch]$NoBuild,
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'PkgPeek')
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$release = Join-Path $root 'target\release'
$files = @(
    @{ From = Join-Path $release 'pkgpeek_shell.dll'; Name = 'pkgpeek_shell.dll' },
    @{ From = Join-Path $release 'pkgpeek.exe'; Name = 'pkgpeek.exe' },
    @{ From = Join-Path $release 'pkgpeek-cli.exe'; Name = 'pkgpeek-cli.exe' },
    @{ From = Join-Path $root 'crates\pkgpeek-shell\PkgPeek.propdesc'; Name = 'PkgPeek.propdesc' }
)

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not $NoBuild) {
    Write-Host 'Compilando (release)...' -ForegroundColor Cyan
    Push-Location $root
    try {
        cargo build --release -p pkgpeek -p pkgpeek-shell
        if ($LASTEXITCODE -ne 0) { throw 'La compilación falló.' }
    } finally { Pop-Location }
}

if (-not (Test-Admin)) {
    Write-Host 'Pidiendo permisos de administrador para registrar la extensión...' -ForegroundColor Cyan
    $shell = (Get-Process -Id $PID).Path
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-NoBuild', '-InstallDir', "`"$InstallDir`"")
    if ($RestartExplorer) { $argList += '-RestartExplorer' }
    $proc = Start-Process $shell -Verb RunAs -ArgumentList $argList -Wait -PassThru
    exit $proc.ExitCode
}

try {
    foreach ($f in $files) {
        if (-not (Test-Path $f.From)) { throw "Falta $($f.From). Compila primero (quita -NoBuild)." }
    }
    New-Item -ItemType Directory -Force $InstallDir | Out-Null

    # Leftovers from previous updates (were locked back then).
    Get-ChildItem $InstallDir -Filter '*.old-*' -ErrorAction SilentlyContinue |
        ForEach-Object { try { Remove-Item $_.FullName -Force -ErrorAction Stop } catch {} }

    foreach ($f in $files) {
        $target = Join-Path $InstallDir $f.Name
        if (Test-Path $target) {
            try {
                Remove-Item $target -Force -ErrorAction Stop
            } catch {
                # Loaded by explorer.exe / prevhost.exe: a loaded DLL can be renamed.
                Rename-Item $target "$($f.Name).old-$(Get-Date -Format yyyyMMddHHmmss)" -Force
            }
        }
        Copy-Item $f.From $target -Force
    }

    $dll = Join-Path $InstallDir 'pkgpeek_shell.dll'
    $reg = Start-Process "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList '/s', "`"$dll`"" -Wait -PassThru
    if ($reg.ExitCode -ne 0) { throw "regsvr32 falló (código $($reg.ExitCode))." }

    if ($RestartExplorer) {
        Get-Process explorer, prevhost -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Milliseconds 800
        $caches = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Explorer'
        Get-ChildItem $caches -Include 'thumbcache_*.db', 'iconcache_*.db' -Recurse -ErrorAction SilentlyContinue |
            ForEach-Object { try { Remove-Item $_.FullName -Force -ErrorAction Stop } catch {} }
        Start-Process explorer.exe
    }

    Write-Host "PKG Peek instalado en $InstallDir" -ForegroundColor Green
    exit 0
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
    Start-Sleep -Seconds 5
    exit 1
}
