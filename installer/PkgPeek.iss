; PKG Peek installer (Inno Setup 6).
; Build with scripts\build-installer.ps1, or:
;   cargo build --release -p pkgpeek -p pkgpeek-shell
;   ISCC.exe /DAppVersion=0.1.0 installer\PkgPeek.iss

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "PKG Peek"
#define AppExe "pkgpeek.exe"
#define AppUrl "https://github.com/m4urigonz/pkg_peek"
#define Root ".."
#define Release Root + "\target\release"

[Setup]
; Never change AppId: it identifies the installation for upgrades.
AppId={{3BC413BC-1906-4F73-A030-C84F3DE39FEA}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=m4urigonz
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={autopf}\PkgPeek
DisableProgramGroupPage=yes
; Machine-wide: the property handler and the property schema live in HKLM.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
ChangesAssociations=yes
; Files held by explorer.exe are moved aside in [Code] instead.
CloseApplications=no
OutputDir={#Root}\dist
OutputBaseFilename=PkgPeek-Setup-{#AppVersion}
SetupIconFile={#Root}\assets\pkg.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
es.RestartExplorer=Reiniciar el Explorador de Windows al terminar (aplica iconos y vista previa de inmediato; cierra sus ventanas)
en.RestartExplorer=Restart Windows Explorer when finished (applies icons and preview right away; closes its windows)
es.UninstallRestartExplorer=Para quitar PKG Peek por completo hay que reiniciar el Explorador de Windows (se cerrarán sus ventanas).%n%n¿Reiniciarlo ahora?%n%nSi eliges No, los archivos restantes se borrarán al reiniciar Windows.
en.UninstallRestartExplorer=To remove PKG Peek completely, Windows Explorer must be restarted (its windows will close).%n%nRestart it now?%n%nIf you choose No, the remaining files will be deleted when Windows restarts.
es.ViewerComment=Ver la información de un PKG de PS4
en.ViewerComment=View the details of a PS4 PKG

[Tasks]
Name: "restartexplorer"; Description: "{cm:RestartExplorer}"; Flags: unchecked

[Files]
; The schema must be in place before the DLL registers itself (it registers
; the schema found next to it).
Source: "{#Root}\crates\pkgpeek-shell\PkgPeek.propdesc"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Release}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Release}\pkgpeek-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Release}\pkgpeek_shell.dll"; DestDir: "{app}"; Flags: ignoreversion regserver uninsrestartdelete
Source: "{#Root}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "{cm:ViewerComment}"

[UninstallDelete]
Type: files; Name: "{app}\*.old-*"

[Code]
const
  MOVEFILE_DELAY_UNTIL_REBOOT = 4;

var
  ExplorerStopped: Boolean;

// lpNewFileName = 0 (NULL) means "delete".
function MoveFileExDelete(lpExistingFileName: String; lpNewFileName: Cardinal; dwFlags: Cardinal): Boolean;
  external 'MoveFileExW@kernel32.dll stdcall';

procedure StopProcess(const Name: String);
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im ' + Name, '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

procedure DeleteOldCopies;
var
  Rec: TFindRec;
  Dir: String;
begin
  Dir := ExpandConstant('{app}\');
  if FindFirst(Dir + '*.old-*', Rec) then
  try
    repeat
      DeleteFile(Dir + Rec.Name);
    until not FindNext(Rec);
  finally
    FindClose(Rec);
  end;
end;

// explorer.exe (icons, thumbnails) and prevhost.exe (preview) keep the DLL
// loaded. A loaded file cannot be overwritten but can be renamed, so the old
// copy is moved aside and deleted later.
procedure MoveAsideIfLocked(const FileName: String);
var
  Path, Aside: String;
begin
  Path := ExpandConstant('{app}\') + FileName;
  if not FileExists(Path) or DeleteFile(Path) then
    Exit;
  Aside := Path + '.old-' + GetDateTimeString('yyyymmddhhnnss', #0, #0);
  if RenameFile(Path, Aside) then
    MoveFileExDelete(Aside, 0, MOVEFILE_DELAY_UNTIL_REBOOT);
end;

// Windows restarts the shell by itself after it is killed (AutoRestartShell).
// Only if that does not happen is it started here, never elevated.
procedure EnsureShellRunning(Uninstalling: Boolean);
var
  I, Code: Integer;
begin
  for I := 1 to 30 do
  begin
    if FindWindowByClassName('Shell_TrayWnd') <> 0 then
      Exit;
    Sleep(200);
  end;
  if Uninstalling then
    Exec(ExpandConstant('{sys}\runas.exe'), '/trustlevel:0x20000 explorer.exe', '', SW_HIDE, ewNoWait, Code)
  else
    ExecAsOriginalUser(ExpandConstant('{win}\explorer.exe'), '', '', SW_SHOW, ewNoWait, Code);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    DeleteOldCopies;
    MoveAsideIfLocked('pkgpeek_shell.dll');
    MoveAsideIfLocked('{#AppExe}');
    MoveAsideIfLocked('pkgpeek-cli.exe');
  end
  else if (CurStep = ssPostInstall) and WizardIsTaskSelected('restartexplorer') then
  begin
    StopProcess('explorer.exe');
    StopProcess('prevhost.exe');
    EnsureShellRunning(False);
  end;
end;

// Files still loaded somewhere (e.g. the search indexer uses the property
// handler) are deleted at the next reboot, followed by the then-empty folder.
procedure ScheduleLeftoversForReboot;
var
  Rec: TFindRec;
  Dir: String;
begin
  Dir := ExpandConstant('{app}');
  if not DirExists(Dir) then
    Exit;
  if FindFirst(Dir + '\*', Rec) then
  try
    repeat
      if Rec.Attributes and FILE_ATTRIBUTE_DIRECTORY = 0 then
        MoveFileExDelete(Dir + '\' + Rec.Name, 0, MOVEFILE_DELAY_UNTIL_REBOOT);
    until not FindNext(Rec);
  finally
    FindClose(Rec);
  end;
  if not RemoveDir(Dir) then
    MoveFileExDelete(Dir, 0, MOVEFILE_DELAY_UNTIL_REBOOT);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    // Silent uninstalls (winget) never close Explorer; locked files are
    // removed at the next reboot instead.
    if not UninstallSilent and
       (MsgBox(CustomMessage('UninstallRestartExplorer'), mbConfirmation, MB_YESNO) = IDYES) then
    begin
      StopProcess('explorer.exe');
      StopProcess('prevhost.exe');
      ExplorerStopped := True;
      Sleep(500);
    end;
  end
  else if CurUninstallStep = usPostUninstall then
  begin
    ScheduleLeftoversForReboot;
    if ExplorerStopped then
      EnsureShellRunning(True);
  end;
end;
