<#
.SYNOPSIS
    Desinstala PKG Peek: quita el registro de la extensión y borra sus archivos.

.DESCRIPTION
    Se eleva a administrador (UAC). El Explorador mantiene la DLL cargada, así
    que por defecto se reinicia para poder borrarla (-KeepExplorer lo evita;
    en ese caso los archivos bloqueados se borran al reiniciar Windows).
#>
param(
    [switch]$KeepExplorer,
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'PkgPeek')
)

$ErrorActionPreference = 'Stop'

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-Admin)) {
    $shell = (Get-Process -Id $PID).Path
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-InstallDir', "`"$InstallDir`"")
    if ($KeepExplorer) { $argList += '-KeepExplorer' }
    $proc = Start-Process $shell -Verb RunAs -ArgumentList $argList -Wait -PassThru
    exit $proc.ExitCode
}

Add-Type -Namespace PkgPeek -Name Native -MemberDefinition @'
[DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
public static extern bool MoveFileEx(string existing, string replacement, int flags);
'@

try {
    $dll = Join-Path $InstallDir 'pkgpeek_shell.dll'
    if (Test-Path $dll) {
        $reg = Start-Process "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList '/u', '/s', "`"$dll`"" -Wait -PassThru
        if ($reg.ExitCode -ne 0) { Write-Warning "regsvr32 /u devolvió $($reg.ExitCode)." }
    }

    if (-not $KeepExplorer) {
        Get-Process explorer, prevhost -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Milliseconds 800
    }

    if (Test-Path $InstallDir) {
        foreach ($f in Get-ChildItem $InstallDir -File) {
            try {
                Remove-Item $f.FullName -Force -ErrorAction Stop
            } catch {
                # MOVEFILE_DELAY_UNTIL_REBOOT
                [void][PkgPeek.Native]::MoveFileEx($f.FullName, $null, 4)
                Write-Warning "$($f.Name) está en uso; se borrará al reiniciar Windows."
            }
        }
        Remove-Item $InstallDir -Force -ErrorAction SilentlyContinue
    }

    if (-not $KeepExplorer) { Start-Process explorer.exe }
    Write-Host 'PKG Peek desinstalado.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
    if (-not $KeepExplorer -and -not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
    Start-Sleep -Seconds 5
    exit 1
}
