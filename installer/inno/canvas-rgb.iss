; canvas-rgb Windows installer (Inno Setup 6). Built by tools/build-installer.ps1:
;   ISCC.exe /DAppVersion=0.1.0 /DPayloadDir=<dist\payload> /DOutDir=<dist> canvas-rgb.iss
;
; The machine-wide work is NOT done by [Files]/[Registry] here. It is the existing, tested PowerShell
; (installer\setup-elevated.ps1 -InnoMode, uninstall-elevated.ps1 -InnoMode); this file collects the
; choices, runs the scripts in a fixed order, and lets Inno own the files, the Start menu shortcut and the
; one Installed apps entry (canvas-rgb_is1).

#ifndef AppVersion
  #define AppVersion "0.4.0"
#endif
; VersionInfoVersion must be numeric x.x.x.x: a test build such as 0.2.1-dev.12 keeps its full string in
; AppVersion and uses only the part before the "-" here.
#define PreIdx Pos("-", AppVersion)
#if PreIdx > 0
  #define NumericVersion Copy(AppVersion, 1, PreIdx - 1)
#else
  #define NumericVersion AppVersion
#endif
#ifndef PayloadDir
  #define PayloadDir "..\..\dist\payload"
#endif
#define PawnIoUrl "https://github.com/namazso/PawnIO"

#ifndef OutDir
  #define OutDir "..\..\dist"
#endif

[Setup]
; Same AppId as the script install's registry key name. Inno always appends _is1 to the key, so the script
; install's key (...\Uninstall\canvas-rgb) cannot be adopted; setup-elevated.ps1 -InnoMode removes it.
AppId=canvas-rgb
AppName=canvas-rgb
AppVersion={#AppVersion}
AppVerName=canvas-rgb {#AppVersion}
AppPublisher=Luke Edwards
AppPublisherURL=https://github.com/iAmChumby/canvas-rgb
AppSupportURL=https://github.com/iAmChumby/canvas-rgb/issues
AppUpdatesURL=https://github.com/iAmChumby/canvas-rgb/releases
AppCopyright=Copyright 2026 Luke Edwards
VersionInfoVersion={#NumericVersion}
DefaultDirName={code:GetDefaultDir}
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\app\src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\canvas-rgb.exe
OutputDir={#OutDir}
OutputBaseFilename=canvas-rgb-setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; setup-elevated.ps1 stops the service, daemon and tray itself; Restart Manager must not touch them.
CloseApplications=no
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Prebuilt binaries (target\release, staged by build-installer.ps1) and the installer scripts.
Source: "{#PayloadDir}\canvas-rgbd.exe";    DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\canvas-rgb.exe";     DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\canvas-rgb-svc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\canvas-rgb-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE";            DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\NOTICE";             DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\installer\*";        DestDir: "{app}\installer"; Flags: ignoreversion
; Extracted to {tmp} before anything is copied: the pre-install checks (preflight, fetch OpenRGB, stop a running install).
Source: "{#PayloadDir}\installer\lib.ps1";            DestDir: "{tmp}"; Flags: dontcopy
Source: "{#PayloadDir}\installer\preflight.ps1";      DestDir: "{tmp}"; Flags: dontcopy
Source: "{#PayloadDir}\installer\detect.ps1";         DestDir: "{tmp}"; Flags: dontcopy
Source: "{#PayloadDir}\installer\stop-canvas.ps1";    DestDir: "{tmp}"; Flags: dontcopy
Source: "{#PayloadDir}\installer\fetch-openrgb.ps1";  DestDir: "{tmp}"; Flags: dontcopy
Source: "{#PayloadDir}\installer\check-install-dir.ps1"; DestDir: "{tmp}"; Flags: dontcopy
Source: "{#PayloadDir}\installer\openrgb.lock.json";  DestDir: "{tmp}"; Flags: dontcopy

[Icons]
; Same path the script install used (CommonPrograms\canvas-rgb.lnk), so an upgrade replaces it.
Name: "{autoprograms}\canvas-rgb"; Filename: "{app}\canvas-rgb.exe"; Comment: "Lighting, fans and peripherals"

[UninstallDelete]
; Made by setup-elevated.ps1 (not by [Files]), so Inno does not know them.
; The OpenRGB tree is NOT here: uninstall-elevated.ps1 deletes it, because it is the only place
; that knows whether an OpenRGB in it is still running. A folder that holds a running OpenRGB is
; kept whole, never deleted half way (#60).
Type: filesandordirs; Name: "{app}\bin"
Type: filesandordirs; Name: "{app}\uninstall"
; Written by a /UPDATE run (see DeinitializeSetup).
Type: files; Name: "{app}\update-result.json"

[Messages]
; Shown by Inno AFTER InitializeUninstall has already stopped canvas-rgb and handed the fans back.
ConfirmUninstall=canvas-rgb has stopped running and every fan is back on your motherboard's own settings.%n%nRemove %1 and all of its files?

[Run]
; Everything else runs from [Code] in a fixed order. This is the only [Run] entry.
; The wizard's ticked "Launch canvas-rgb" starts the tray with --onboard, so the window opens on the
; onboarding. skipifsilent: a /SILENT run starts the tray from [Code] instead (CurStepChanged), so the
; two paths never start it twice. A /VERYSILENT run starts nothing.
Filename: "{app}\canvas-rgb.exe"; Parameters: "--onboard"; Description: "Launch canvas-rgb"; Flags: nowait postinstall skipifsilent runasoriginaluser; Check: ScriptsOk

[Code]
const
  UninstallSubkey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\canvas-rgb';
  // PowerShell that exits 0 when %APP% is under %PF% (case-insensitive, whole path segments), else 1. Single
  // quotes are doubled for Pascal; the install-folder tests run this very text for both branches.
  UnderProgramFilesScript = '$a = ([IO.DirectoryInfo]''%APP%'').FullName.TrimEnd(''\'') + ''\''; $b = ([IO.DirectoryInfo]''%PF%'').FullName.TrimEnd(''\'') + ''\''; if ($a.StartsWith($b, [StringComparison]::OrdinalIgnoreCase)) { exit(0) } else { exit(1) }';
  // PowerShell run BEFORE anything in {app} when the protected check copy is missing: it checks {app}, its
  // installer folder, the two scripts the uninstaller runs and the program, with code that does not come from
  // {app}. Exits 1 on a junction, an owner other than SYSTEM or Administrators, a NULL permission list, or a
  // write right given to anyone else. The install-folder tests run this very text on throwaway folders.
  AppCheckScript = '$k = ''S-1-5-18'',''S-1-5-32-544''; $t = ''S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464''; $w = 0x4 -bor 0x2 -bor 0x10 -bor 0x40 -bor 0x100 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x40000000 -bor 0x10000000; $r = ([IO.DirectoryInfo]''%APP%'').FullName.TrimEnd(''\''); $bad = 0; foreach ($f in $r, ($r + ''\installer''), ($r + ''\installer\uninstall-elev'' + ''ated.ps1''), ($r + ''\installer\lib.ps1''), ($r + ''\installer\uninstall-user.ps1''), ($r + ''\canvas-rgb-svc.exe'')) { if (-not ([IO.File]::Exists($f) -or [IO.Directory]::Exists($f))) { continue }; if ([IO.File]::GetAttributes($f) -band 0x400) { $bad = 1 }; $a = Get-Acl -LiteralPath $f; if ($a.GetSecurityDescriptorSddlForm(''Access'') -eq '''') { $bad = 1 }; if ($k -notcontains $a.GetOwner([Security.Principal.SecurityIdentifier]).Value) { $bad = 1 }; foreach ($x in $a.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) { if ($x.AccessControlType -eq ''Allow'' -and -not ($x.PropagationFlags -band 2) -and (([int]$x.FileSystemRights -band $w) -ne 0) -and ($k + $t) -notcontains $x.IdentityReference.Value) { $bad = 1 } } }; exit($bad)';
  // PowerShell that starts canvas-rgb again after an update that failed before the elevated step
  // ran (#65). It calls the same code the scripts use (lib.ps1), so a service that will not start
  // is handled the same way. %LIB% is the lib.ps1 that was extracted to {tmp}.
  RestartServiceScript = '. ''%LIB%''; Start-CanvasRgbService -WasRunning $true';

var
  SignalRgbPage: TInputOptionWizardPage;
  PawnChoicePage: TInputOptionWizardPage;
  PawnFilePage: TInputFileWizardPage;
  DetPawnIo, DetPawnMissing, DetSgInstalled, DetSgAuto, DetSgRunning: String;
  FScriptsOk: Boolean;
  FOpenRgbZip: AnsiString;
  FFailCode: Integer;       // 0 = ok; else the exit code Setup returns (GetCustomSetupExitCode)
  FFailText: String;
  FPrepError: String;       // why PrepareToInstall stopped Setup, for the /UPDATE result file
  FPurge: String;           // ' -PurgeConfig' or empty
  FStoppedRunningService: Boolean;  // the first step stopped a running service (#60)
  FElevatedStepRan: Boolean;       // the elevated step ran: it starts the service itself when it fails (#65)
  FInstallDirOk: Boolean;           // the uninstall check passed: the folder only administrators can change (#46)

function ScriptsOk: Boolean;
begin
  Result := FScriptsOk;
end;

function GetParam(const Name, Default: String): String;
var
  I: Integer;
  S, Prefix: String;
begin
  Result := Default;
  Prefix := '/' + Uppercase(Name) + '=';
  for I := 1 to ParamCount do
  begin
    S := ParamStr(I);
    if Pos(Prefix, Uppercase(S)) = 1 then
    begin
      Result := Copy(S, Length(Prefix) + 1, MaxInt);
      Exit;
    end;
  end;
end;

// True for /VERYSILENT. Inno has WizardSilent for both /SILENT and /VERYSILENT, and no separate test.
function IsVerySilent: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if Uppercase(ParamStr(I)) = '/VERYSILENT' then Result := True;
end;

// True for /UPDATE, the switch the tray passes when it runs this setup to update itself.
function IsUpdateRun: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if Uppercase(ParamStr(I)) = '/UPDATE' then Result := True;
end;

// True for /SERVICEUPDATE, the switch the canvas-rgb service passes when it runs this setup itself (#43).
// Setup then runs as SYSTEM, so "the original user" is SYSTEM, not the person at the keyboard: the per-user
// script and the tray relaunch are skipped here, and the service does both as the console user once
// setup ends (setup-user.ps1, then canvas-rgb.exe --after-update).
function IsServiceUpdateRun: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if Uppercase(ParamStr(I)) = '/SERVICEUPDATE' then Result := True;
end;

function PsExe: String;
begin
  Result := ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe');
end;

// True when {app} is inside Program Files ({commonpf}). Used only when the protected check copy is missing.
function AppIsUnderProgramFiles: Boolean;
var
  Code: Integer;
  App, Pf, Script: String;
begin
  App := ExpandConstant('{app}');
  Pf := ExpandConstant('{commonpf}');
  StringChangeEx(App, '''', '''''', True);
  StringChangeEx(Pf, '''', '''''', True);
  Script := UnderProgramFilesScript;
  StringChangeEx(Script, '%APP%', App, True);
  StringChangeEx(Script, '%PF%', Pf, True);
  // token: Exec runs as Setup's own administrator token; the script only compares two paths.
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'), '-NoProfile -Command "' + Script + '"',
    ExpandConstant('{sys}'), SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

// Runs AppCheckScript on {app}: True when it found nothing wrong. Code is its exit code.
function AppFolderPassesCheck(var Code: Integer): Boolean;
var
  App, Script: String;
begin
  App := ExpandConstant('{app}');
  StringChangeEx(App, '''', '''''', True);
  Script := AppCheckScript;
  StringChangeEx(Script, '%APP%', App, True);
  // token: Exec runs as Setup's own administrator token; the script only reads owners and permissions.
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'), '-NoProfile -Command "' + Script + '"',
    ExpandConstant('{sys}'), SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

function RunPs(const ScriptPath, Args: String; AsOriginalUser: Boolean; var Code: Integer): Boolean;
var
  Params: String;
begin
  Params := '-NoProfile -ExecutionPolicy Bypass -File "' + ScriptPath + '" ' + Args;
  // token: ExecAsOriginalUser runs as the person who started Setup (non-elevated); Exec runs as Setup's own
  // administrator token.
  if AsOriginalUser then
    Result := ExecAsOriginalUser(PsExe, Params, ExpandConstant('{sys}'), SW_HIDE, ewWaitUntilTerminated, Code)
  else
    Result := Exec(PsExe, Params, ExpandConstant('{sys}'), SW_HIDE, ewWaitUntilTerminated, Code);
end;

// Starts canvas-rgb again after an update that failed before the elevated step ran (#65): the
// first step stopped the service before it copied anything, and nothing has started it again, so
// the fans would stay on the motherboard's own settings. It uses the same code as the scripts
// (lib.ps1), so a service that will not start is handled the same way.
function RestartCanvasRgb: Boolean;
var
  Code: Integer;
  Script, Lib: String;
begin
  Result := False;
  Lib := ExpandConstant('{tmp}\lib.ps1');
  if not FileExists(Lib) then Exit;
  StringChangeEx(Lib, '''', '''''', True);
  Script := RestartServiceScript;
  StringChangeEx(Script, '%LIB%', Lib, True);
  // token: Exec runs as Setup's own administrator token; the script only starts the service.
  Result := Exec(PsExe, '-NoProfile -ExecutionPolicy Bypass -Command "' + Script + '"',
    ExpandConstant('{sys}'), SW_HIDE, ewWaitUntilTerminated, Code);
end;

function LogTail(const LogPath: String): String;
var
  Text: AnsiString;
begin
  Result := '';
  if LoadStringFromFile(LogPath, Text) then
  begin
    { The log is UTF-8 (the PowerShell scripts write it as UTF-8 without a BOM). Decoded as UTF-8,
      so the message the person reads is the text that was written: read as the system code page
      a BOM showed up as three odd characters, and every accent was wrong (#60). }
    Result := Utf8Decode(Text);
    if (Length(Result) > 0) and (Result[1] = #65279) then
      Result := Copy(Result, 2, MaxInt);
    if Length(Result) > 1200 then Result := '...' + Copy(Result, Length(Result) - 1199, 1200);
  end;
end;

// An existing script install lives where its InstallLocation says. Inno installs are found by UsePreviousAppDir.
function GetDefaultDir(Param: String): String;
var
  Loc: String;
begin
  Result := ExpandConstant('{autopf}\canvas-rgb');
  if RegQueryStringValue(HKLM64, UninstallSubkey, 'InstallLocation', Loc) and (Loc <> '') then
    Result := Loc;
end;

// ---- what is already on this PC (detect.ps1, read-only) ----

procedure ReadDetection;
var
  Lines: TArrayOfString;
  I, P, Code: Integer;
  OutPath, Line: String;
  Ran: Boolean;
begin
  DetPawnIo := 'Unknown';
  DetPawnMissing := '';
  DetSgInstalled := 'Unknown';
  DetSgAuto := '0';
  DetSgRunning := '0';
  ExtractTemporaryFile('lib.ps1');
  ExtractTemporaryFile('detect.ps1');
  OutPath := ExpandConstant('{tmp}\detect.txt');
  Ran := False;
  try
    // As the logged-in user first: SignalRGB's start-up entries belong to that account.
    Ran := RunPs(ExpandConstant('{tmp}\detect.ps1'), '-OutFile "' + OutPath + '"', True, Code) and FileExists(OutPath);
  except
    Ran := False;
  end;
  if not Ran then Ran := RunPs(ExpandConstant('{tmp}\detect.ps1'), '-OutFile "' + OutPath + '"', False, Code) and FileExists(OutPath);
  if (not Ran) or (not LoadStringsFromFile(OutPath, Lines)) then Exit;
  for I := 0 to GetArrayLength(Lines) - 1 do
  begin
    Line := Lines[I];
    P := Pos('=', Line);
    if P = 0 then Continue;
    if Copy(Line, 1, P - 1) = 'pawnio' then DetPawnIo := Copy(Line, P + 1, MaxInt);
    if Copy(Line, 1, P - 1) = 'pawnio_missing' then DetPawnMissing := Copy(Line, P + 1, MaxInt);
    if Copy(Line, 1, P - 1) = 'signalrgb_installed' then
    begin
      DetSgInstalled := Copy(Line, P + 1, MaxInt);
      if DetSgInstalled = '1' then DetSgInstalled := 'Yes'
      else if DetSgInstalled = '0' then DetSgInstalled := 'No';
    end;
    if Copy(Line, 1, P - 1) = 'signalrgb_autostarts' then DetSgAuto := Copy(Line, P + 1, MaxInt);
    if Copy(Line, 1, P - 1) = 'signalrgb_running' then DetSgRunning := Copy(Line, P + 1, MaxInt);
  end;
end;

function PawnIoIsReady: Boolean;
begin
  Result := (DetPawnIo = 'Ready') or (DetPawnIo = 'NeedsModules');
end;

// The silent switches win; in a silent run nothing is taken from the pages (they never showed) and
// nothing about SignalRGB changes unless /SIGNALRGB=Y is given.
function GetSignalRgb: String;
begin
  Result := 'N';
  if Uppercase(GetParam('SIGNALRGB', 'N')) = 'Y' then Result := 'Y'
  else if (not WizardSilent) and (SignalRgbPage <> nil) and (DetSgInstalled <> 'No') and SignalRgbPage.Values[0] then Result := 'Y';
end;

function GetPawnIoPath: String;
begin
  Result := GetParam('PAWNIO', '');
  if (Result = '') and (not WizardSilent) and (not PawnIoIsReady) and (PawnChoicePage <> nil) and (PawnFilePage <> nil) then
    if PawnChoicePage.Values[1] then Result := Trim(PawnFilePage.Values[0]);
end;

procedure OpenPawnIoPageClick(Sender: TObject);
var
  Code: Integer;
begin
  // Opened as the logged-in user (not with Setup's administrator rights).
  // token: the original user's, non-elevated.
  ExecAsOriginalUser(ExpandConstant('{win}\explorer.exe'), '{#PawnIoUrl}', '', SW_SHOWNORMAL, ewNoWait, Code);
end;

procedure InitializeWizard;
var
  Intro, SgLabel: String;
  Btn: TNewButton;
begin
  ReadDetection;

  // SignalRGB: skipped when it is not installed.
  Intro := 'Both programs control the same lights, so if both run at once they fight over the colours. ' +
    'canvas-rgb can stop SignalRGB from starting with Windows. Nothing is uninstalled, and uninstalling canvas-rgb ' +
    'puts SignalRGB''s start-up setting back the way it was.';
  if DetSgInstalled = 'Unknown' then
    Intro := 'We could not check whether SignalRGB is installed on this PC. ' + Intro
  else if DetSgAuto = '1' then
    Intro := Intro + #13#10 + #13#10 + 'SignalRGB is set to start with Windows right now.'
  else
    Intro := Intro + #13#10 + #13#10 + 'SignalRGB is already set not to start with Windows, so there is nothing to switch off.';
  SgLabel := 'Stop SignalRGB from starting with Windows';
  if DetSgRunning = '1' then
  begin
    Intro := Intro + #13#10 + #13#10 + 'SignalRGB is running right now. If you tick the box, setup closes it. If you leave it unticked, it keeps running.';
    SgLabel := SgLabel + ' and close it now';
  end;
  SignalRgbPage := CreateInputOptionPage(wpSelectDir, 'SignalRGB is installed on this PC',
    'Choose what happens to SignalRGB', Intro, False, False);
  SignalRgbPage.Add(SgLabel);
  SignalRgbPage.Values[0] := (DetSgAuto = '1') and (DetSgInstalled = 'Yes');

  // Fan control: no question when PawnIO is already there.
  Intro := 'canvas-rgb can control the fans in your case. To do that it needs a small free driver called PawnIO';
  if DetPawnIo = 'Unknown' then
    Intro := Intro + '. We could not check whether it is installed on this PC.'
  else
    Intro := Intro + ', which is not installed on this PC yet.';
  Intro := Intro + ' Without it, lighting and everything else still work, and your fan speeds stay on your motherboard''s ' +
    'own settings. You can add PawnIO later by running this setup again.';
  PawnChoicePage := CreateInputOptionPage(SignalRgbPage.ID, 'Fan control', 'Do you want canvas-rgb to control your fans?',
    Intro, True, False);
  PawnChoicePage.Add('Skip fan control for now');
  PawnChoicePage.Add('Set up fan control now, using the PawnIO installer I have downloaded');
  PawnChoicePage.Values[0] := True;
  PawnChoicePage.CheckListBox.Height := ScaleY(60);
  Btn := TNewButton.Create(PawnChoicePage);
  Btn.Parent := PawnChoicePage.Surface;
  Btn.Caption := 'Open the PawnIO download page';
  Btn.Left := 0;
  Btn.Top := PawnChoicePage.CheckListBox.Top + PawnChoicePage.CheckListBox.Height + ScaleY(12);
  Btn.Width := ScaleX(200);
  Btn.Height := ScaleY(26);
  Btn.OnClick := @OpenPawnIoPageClick;

  PawnFilePage := CreateInputFilePage(PawnChoicePage.ID, 'Choose the PawnIO installer',
    'Pick the file you downloaded',
    'Choose the PawnIO installer you downloaded (the file ends in .exe). Setup checks that it is genuinely signed ' +
    'before it runs it, and refuses it if it is not.');
  PawnFilePage.Add('PawnIO installer:', 'Programs (*.exe)|*.exe|All files (*.*)|*.*', '.exe');
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := False;
  if (SignalRgbPage <> nil) and (PageID = SignalRgbPage.ID) then Result := (DetSgInstalled = 'No');
  if (PawnChoicePage <> nil) and (PageID = PawnChoicePage.ID) then Result := PawnIoIsReady;
  if (PawnFilePage <> nil) and (PageID = PawnFilePage.ID) then Result := PawnIoIsReady or (not PawnChoicePage.Values[1]);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (PawnFilePage <> nil) and (CurPageID = PawnFilePage.ID) then
  begin
    if Trim(PawnFilePage.Values[0]) = '' then
    begin
      MsgBox('Pick the PawnIO installer you downloaded, or go Back and choose to skip fan control for now.', mbError, MB_OK);
      Result := False;
    end
    else if not FileExists(Trim(PawnFilePage.Values[0])) then
    begin
      MsgBox('That file does not exist. Pick the PawnIO installer you downloaded, or go Back and choose to skip fan control for now.', mbError, MB_OK);
      Result := False;
    end;
  end;
end;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo, MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := MemoDirInfo + NewLine + NewLine + 'Fan control:' + NewLine;
  if DetPawnIo = 'Ready' then
    Result := Result + Space + 'Ready. PawnIO is already installed.' + NewLine
  else if DetPawnIo = 'NeedsModules' then
    Result := Result + Space + 'PawnIO is installed, but these files canvas-rgb needs for fans are missing: ' + DetPawnMissing +
      '. Until they are added, fan speeds stay on your motherboard''s own settings.' + NewLine
  else if GetPawnIoPath <> '' then
    Result := Result + Space + 'Set up now, using ' + GetPawnIoPath + NewLine
  else
    Result := Result + Space + 'Skipped for now. Fan speeds stay on your motherboard''s own settings.' + NewLine;

  Result := Result + NewLine + 'SignalRGB:' + NewLine;
  if DetSgInstalled = 'No' then
    Result := Result + Space + 'Not installed on this PC. Nothing to do.' + NewLine
  else if GetSignalRgb = 'Y' then
  begin
    Result := Result + Space + 'Stop it from starting with Windows';
    if DetSgRunning = '1' then Result := Result + ' and close it now';
    Result := Result + NewLine;
  end
  else
    Result := Result + Space + 'Leave it as it is.' + NewLine;
end;

// Remembers why Setup stopped before copying, then returns it for PrepareToInstall.
function PrepFail(const Text: String): String;
begin
  FPrepError := Text;
  Result := Text;
end;

// The marker the first step (stop-canvas.ps1) wrote when it stopped a running service. It lives
// in the protected machine home, so only an administrator can write one.
function StoppedServiceMarkerPath: String;
begin
  Result := ExpandConstant('{commonappdata}\canvas-rgb\stopped-service.marker');
end;

// True when the marker says this run stopped a running service. The elevated step runs after the
// files were copied and finds the service already stopped, so it cannot know that for itself: the
// switch below hands the fact over, and a failure there starts the service again (#60).
function StoppedRunningService: Boolean;
var
  Text: AnsiString;
begin
  Result := False;
  if LoadStringFromFile(StoppedServiceMarkerPath, Text) then
    Result := Pos('stoppedByThisRun=yes', Utf8Decode(Text)) > 0;
end;

// The state an unfinished uninstall left behind (#64). The uninstall script writes it in the
// protected machine home: it says in plain words what was already removed, so the message below
// is true instead of "nothing was removed".
function UninstallStatePath: String;
begin
  Result := ExpandConstant('{commonappdata}\canvas-rgb\uninstall-state.marker');
end;

// The words the uninstall script left behind, or '' when there is none.
function UninstallStateWords: String;
var
  Text: AnsiString;
begin
  Result := '';
  if LoadStringFromFile(UninstallStatePath, Text) then
  begin
    Result := Utf8Decode(Text);
    if (Length(Result) > 0) and (Result[1] = #65279) then Result := Copy(Result, 2, MaxInt);
    Result := Trim(Result);
  end;
end;

// Before any file is copied: fetch and verify OpenRGB (nothing is changed if this fails), then stop a running install.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Dl, Log, DirLog, Args: String;
begin
  Result := '';
  Log := ExpandConstant('{tmp}\canvas-rgb-prepare.log');
  ExtractTemporaryFile('lib.ps1');
  ExtractTemporaryFile('check-install-dir.ps1');
  ExtractTemporaryFile('preflight.ps1');
  ExtractTemporaryFile('stop-canvas.ps1');
  ExtractTemporaryFile('fetch-openrgb.ps1');
  ExtractTemporaryFile('openrgb.lock.json');

  // Every check that can fail early runs here, before any file is copied. A non-empty result stops
  // Setup on the Preparing to Install page with exit code 7 ("cannot proceed with installation").
  WizardForm.PreparingLabel.Caption := 'Checking this PC...';
  Args := '-LogFile "' + Log + '"';
  if GetPawnIoPath <> '' then Args := Args + ' -PawnIoInstaller "' + GetPawnIoPath + '"';
  if not RunPs(ExpandConstant('{tmp}\preflight.ps1'), Args, False, Code) or (Code <> 0) then
  begin
    Result := PrepFail('Setup found a problem before copying anything. Nothing was changed.' + #13#10 + #13#10 + LogTail(Log));
    Exit;
  end;

  // The install folder runs code as SYSTEM (the service, OpenRGB, these scripts), so it must be a folder only
  // administrators can change. This locks it first, then refuses on anything a user could write, before
  // any file is copied and before anything is downloaded or stopped (#46). A refusal never deletes or
  // takes over a thing; it names the folder and what is wrong.
  WizardForm.PreparingLabel.Caption := 'Checking the install folder...';
  DirLog := ExpandConstant('{tmp}\canvas-rgb-dircheck.log');
  if not RunPs(ExpandConstant('{tmp}\check-install-dir.ps1'), '-InstallDir "' + ExpandConstant('{app}') + '" -Mode Install -LogFile "' + DirLog + '"', False, Code) or (Code <> 0) then
  begin
    Result := PrepFail('Setup cannot use this install folder, and nothing was copied.' + #13#10 + #13#10 + LogTail(DirLog));
    Exit;
  end;

  WizardForm.PreparingLabel.Caption := 'Downloading the lighting component (OpenRGB) and checking it is genuine...';
  Dl := ExpandConstant('{tmp}\openrgb-dl');
  if not RunPs(ExpandConstant('{tmp}\fetch-openrgb.ps1'), '-OutDir "' + Dl + '"', False, Code) or (Code <> 0) then
  begin
    Result := PrepFail('Setup could not download the lighting component (OpenRGB), or it did not pass its safety check (code ' + IntToStr(Code) + '). Nothing was changed. Check your internet connection and run setup again.');
    Exit;
  end;
  if not LoadStringFromFile(Dl + '\zip-path.txt', FOpenRgbZip) then
  begin
    Result := PrepFail('The lighting component (OpenRGB) download left no file behind. Nothing was changed.');
    Exit;
  end;

  WizardForm.PreparingLabel.Caption := 'Closing canvas-rgb if it is running...';
  // A marker from an earlier run says nothing about this one.
  DeleteFile(StoppedServiceMarkerPath);
  if not RunPs(ExpandConstant('{tmp}\stop-canvas.ps1'), '-LogFile "' + Log + '" -InstallDir "' + ExpandConstant('{app}') + '"', False, Code) or (Code <> 0) then
  begin
    // Nothing was copied: the checks above all ran before a single file was touched. What was
    // stopped and what was not is in the log.
    Result := PrepFail('Setup could not close everything it has to replace (code ' + IntToStr(Code) + '). Nothing was copied.' + #13#10 + #13#10 + LogTail(Log));
    Exit;
  end;

  // Did that stop a running service? The marker in the machine home says so, and the switch below
  // hands the fact to the elevated step: it runs after the files were copied, finds the service
  // already stopped, and cannot see that for itself (#60).
  FStoppedRunningService := StoppedRunningService();
  DeleteFile(StoppedServiceMarkerPath);

  // A result file from an earlier run says nothing about this one. It is deleted last, so a
  // refusal above leaves the install folder exactly as it was.
  DeleteFile(ExpandConstant('{app}\update-result.json'));
end;

// After the files are in {app}: the elevated machine-wide script, then the per-user script as the logged-in
// user (ExecAsOriginalUser: HKCU and %APPDATA% belong to the person who started setup), then the daemon.
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
  Log, Args, Pawn: String;
begin
  if CurStep <> ssPostInstall then Exit;
  // The elevated step runs from here on. When it fails it starts the service again itself (it is
  // told what the first step stopped), so the restart in DeinitializeSetup must not start it a
  // second time (#65).
  FElevatedStepRan := True;
  Log := ExpandConstant('{tmp}\canvas-rgb-setup-elevated.log');
  Args := '-PayloadDir "' + ExpandConstant('{app}') + '" -InstallDir "' + ExpandConstant('{app}') + '" -InnoMode' +
          ' -OpenRgbZip "' + Trim(String(FOpenRgbZip)) + '" -DisableSignalRgb ' + GetSignalRgb + ' -LogFile "' + Log + '"';
  Pawn := GetPawnIoPath;
  if Pawn <> '' then Args := Args + ' -PawnIoInstaller "' + Pawn + '"';
  // The first step stopped a running service before the files were copied. This step finds the
  // service stopped and cannot see that, so the switch hands it over: a failure here starts the
  // service again instead of leaving canvas-rgb down with the fans on the motherboard's own
  // settings (#60).
  if FStoppedRunningService then Args := Args + ' -StoppedRunningService';

  WizardForm.StatusLabel.Caption := 'Setting up canvas-rgb to run in the background...';
  if not RunPs(ExpandConstant('{app}\installer\setup-elevated.ps1'), Args, False, Code) or (Code <> 0) then
  begin
    FFailCode := 10;
    FFailText := 'Setup could not finish installing the background part of canvas-rgb (code ' + IntToStr(Code) + '). It may not be running, and your account settings were not set up. Run this setup again.' + #13#10 + #13#10 + LogTail(Log);
    SuppressibleMsgBox(FFailText, mbError, MB_OK, IDOK);
    Exit;
  end;

  { /SERVICEUPDATE: setup runs as SYSTEM, so ExecAsOriginalUser would run the per-user script as SYSTEM and
    write SYSTEM's HKCU and profile. The service runs setup-user.ps1 as the console user after setup ends. The
    elevated script above still counts, so the run is ok. }
  if not IsServiceUpdateRun then
  begin
    WizardForm.StatusLabel.Caption := 'Setting up canvas-rgb for your account...';
    Args := '-InstallDir "' + ExpandConstant('{app}') + '" -SignalRgb ' + GetSignalRgb;
    if not RunPs(ExpandConstant('{app}\installer\setup-user.ps1'), Args, True, Code) or (Code <> 0) then
    begin
      FFailCode := 11;
      FFailText := 'Setup could not set up canvas-rgb for your account (code ' + IntToStr(Code) + '). The background part is installed. Run this setup again to finish.';
      SuppressibleMsgBox(FFailText, mbError, MB_OK, IDOK);
      Exit;
    end;
  end;
  FScriptsOk := True;
  { The service owns launching the daemon (in the user's session). Starting canvas-rgbd.exe here raced it:
    the service's daemon then failed to bind the API (os error 10048) and was refused as not the launched one. }
  { The tray, in the user's session, on the onboarding. Every mode except /VERYSILENT starts it once:
    the wizard does it through the ticked "Launch canvas-rgb" ([Run], --onboard), /SILENT does it here
    ([Run] has skipifsilent). The tray is also single-instance, so a stray second launch only focuses it. }
  { /UPDATE (the tray updating itself) relaunches it with --after-update: it starts quietly, with no window. }
  // token: ExecAsOriginalUser starts the tray as the person who started Setup. That is the non-elevated user only
  // because the tray starts Setup unelevated and Setup raises itself (#28): never start Setup with an elevating verb.
  { /SERVICEUPDATE never relaunches the tray from here (the original user is SYSTEM): the service starts it. }
  if WizardSilent and (not IsVerySilent) and (not IsServiceUpdateRun) then
  begin
    if IsUpdateRun then
      ExecAsOriginalUser(ExpandConstant('{app}\canvas-rgb.exe'), '--after-update', ExpandConstant('{app}'), SW_SHOWNORMAL, ewNoWait, Code)
    else
      ExecAsOriginalUser(ExpandConstant('{app}\canvas-rgb.exe'), '--onboard', ExpandConstant('{app}'), SW_SHOWNORMAL, ewNoWait, Code);
  end;
end;

// A post-copy failure cannot undo the copied files (Abort there is not documented to exit Setup, and a
// rollback would delete the exes the service points at). Setup instead ends on a failure page with a
// non-zero exit code: GetCustomSetupExitCode is called when Setup ran to completion and would return 0.
function GetCustomSetupExitCode: Integer;
begin
  Result := FFailCode;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpFinished) and (FFailCode <> 0) then
  begin
    WizardForm.FinishedHeadingLabel.Caption := 'canvas-rgb setup did not finish';
    WizardForm.FinishedLabel.Caption := FFailText;
  end;
end;

// Escapes a string for the result file (a JSON string without its quotes): backslash, quote, and every
// character below 0x20 as \u00XX.
function JsonEscape(const S: String): String;
var
  I, N: Integer;
  C: String;
begin
  Result := '';
  for I := 1 to Length(S) do
  begin
    C := Copy(S, I, 1);
    N := Ord(S[I]);
    if (C = '\') or (C = '"') then Result := Result + '\' + C
    else if N < 32 then Result := Result + '\u00' + Copy('0123456789abcdef', (N div 16) + 1, 1) + Copy('0123456789abcdef', (N mod 16) + 1, 1)
    else Result := Result + C;
  end;
end;

// /UPDATE only. The tray that started this run is gone (stop-canvas.ps1 ends it), so the outcome goes in a
// file the next tray reads: {app}\update-result.json. It lives in {app} (admin-writable, user-readable)
// rather than in a user's profile: a profile folder here belongs to whoever approved the permission
// prompt, which is another account when a standard user typed an admin's password. On a failure the tray
// is started again (--after-update, quietly) so it can show the message; a tray that is still running
// ignores the second launch.
procedure DeinitializeSetup;
var
  Code: Integer;
  Ok: Boolean;
  Msg, Json, Exe: String;
  Lines: TArrayOfString;
begin
  try
    Ok := FScriptsOk;
    { The install did not finish, and this run stopped a running service before it copied anything
      (the first step did, so every fan is back on the motherboard's own settings). Nothing has
      started it again: the elevated step never ran, so it could not. Start it now, so an update
      that failed or was cancelled does not leave the fans on BIOS (#65).
      A service the user had stopped on purpose is not started: the first step writes the marker
      only when it stopped a running one. }
    if (not Ok) and FStoppedRunningService and (not FElevatedStepRan) and (not IsServiceUpdateRun) then
      RestartCanvasRgb;
    if not IsUpdateRun then Exit;
    Msg := '';
    if not Ok then
    begin
      if FFailText <> '' then Msg := FFailText
      else if FPrepError <> '' then Msg := FPrepError
      else Msg := 'Setup did not finish.';
    end;
    Json := '{"version":"{#AppVersion}","ok":';
    if Ok then Json := Json + 'true' else Json := Json + 'false';
    Json := Json + ',"code":' + IntToStr(FFailCode) + ',"message":"' + JsonEscape(Msg) + '"}';
    // The plain save-string function takes an AnsiString (jrsoftware.org/ishelp/topic_scriptfunctions.htm),
    // so a non-ASCII message would be cut to the system code page. This writes UTF-8 with no BOM.
    SetArrayLength(Lines, 1);
    Lines[0] := Json;
    SaveStringsToUTF8FileWithoutBOM(ExpandConstant('{app}\update-result.json'), Lines, False);
    Exe := ExpandConstant('{app}\canvas-rgb.exe');
    // token: the original user's, non-elevated (see CurStepChanged).
    // /SERVICEUPDATE: the original user is SYSTEM, so no relaunch here; the service starts the tray.
    // /SERVICEUPDATE that did not finish: setup-elevated.ps1 may have stopped the service and failed before
    // it started the new one, which leaves nobody to start the tray. Start the service; it finds the marker,
    // runs setup-user.ps1 and starts the tray, which shows update-result.json. The result is ignored (the
    // service may already be running: error 1056).
    // token: Setup's own (SYSTEM for a service update); sc.exe only starts the service.
    if (not Ok) and IsServiceUpdateRun then
      Exec(ExpandConstant('{sys}\sc.exe'), 'start canvas-rgb-svc', '', SW_HIDE, ewWaitUntilTerminated, Code);
    if (not Ok) and FileExists(Exe) and (not IsServiceUpdateRun) then
      ExecAsOriginalUser(Exe, '--after-update', ExpandConstant('{app}'), SW_SHOWNORMAL, ewNoWait, Code);
  except
    // The install folder is not set when Setup stopped very early: there is nothing to report to.
  end;
end;

// ---- uninstall: hand the fans to BIOS first, then Inno removes the files ----
// InitializeUninstall is the documented abort point ("Return False to abort Uninstall, True otherwise").
// In Inno's source it runs in the elevated second phase (the uninstaller respawns itself elevated, and a
// non-admin second phase aborts), before any file removal. It runs before Inno's own confirmation, so it
// asks first, and Inno's confirmation text (see [Messages]) says what has already happened.
function InitializeUninstall: Boolean;
var
  Code: Integer;
  Log, Args, CheckLog, CheckScript, Msg: String;
  Ran: Boolean;
begin
  Result := True;
  FPurge := '';
  FInstallDirOk := False;
  // First of all, before anything in the install folder is run (the elevated script below runs
  // canvas-rgb-svc.exe as an administrator): the folder must be one only administrators can change.
  // The check runs from a protected copy in the machine home, never from {app}, which is the folder
  // being checked. An install made before this check was added has no copy: its folder is then checked by
  // the script embedded below (AppCheckScript), never by code from {app}, and refused when it is not
  // admin-only, with a message that says how to fix it (#46).
  CheckLog := ExpandConstant('{tmp}\canvas-rgb-uninstall-dircheck.log');
  CheckScript := ExpandConstant('{commonappdata}\canvas-rgb\check\check-install-dir.ps1');
  if not FileExists(CheckScript) then
  begin
    if AppIsUnderProgramFiles then
    begin
      // The protected copy is gone. Before uninstall-elevated.ps1 (which lives in {app}) runs, {app} is
      // checked with code that is not from {app}: AppCheckScript.
      Ran := AppFolderPassesCheck(Code);
      if not Ran then
        SaveStringToFile(CheckLog, 'canvas-rgb cannot confirm that only administrators can change the folder it is installed in, or the check could not run. Nothing was removed or run. Fix the permissions on that folder yourself, or install canvas-rgb again into the default folder under Program Files and remove the old folder by hand.', False);
    end
    else
    begin
      SaveStringToFile(CheckLog, 'This copy of canvas-rgb is in a folder outside Program Files, and the safety check that setup keeps in the protected canvas-rgb data folder is missing, so the uninstaller cannot confirm that only administrators can change this folder. Nothing was removed or run. Install canvas-rgb again into the default folder under Program Files (the new install takes over from this one), then remove this old folder by hand.', False);
      Ran := False;
      Code := 1;
    end;
  end
  else
    Ran := RunPs(CheckScript, '-InstallDir "' + ExpandConstant('{app}') + '" -Mode Verify -LogFile "' + CheckLog + '"', False, Code);
  if not (Ran and (Code = 0)) then
  begin
    if not UninstallSilent then
      MsgBox('canvas-rgb cannot be removed from this folder.' + #13#10 + #13#10 + LogTail(CheckLog), mbError, MB_OK);
    Result := False;
    Exit;
  end;
  FInstallDirOk := True;
  if UninstallSilent then
  begin
    if Uppercase(GetParam('PURGE', 'N')) = 'Y' then FPurge := ' -PurgeConfig';
  end
  else
  begin
    if MsgBox('Remove canvas-rgb now?' + #13#10 + #13#10 +
        'First it stops running and gives every fan back to your motherboard''s own settings. If that fails, nothing is deleted.',
        mbConfirmation, MB_YESNO or MB_DEFBUTTON2) <> IDYES then
    begin
      Result := False;
      Exit;
    end;
    if MsgBox('Also delete your canvas-rgb settings and saved fan settings?' + #13#10 +
        'Choose No to keep them for a later reinstall.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      FPurge := ' -PurgeConfig';
  end;

  Log := ExpandConstant('{tmp}\canvas-rgb-uninstall-elevated.log');
  Args := '-InstallDir "' + ExpandConstant('{app}') + '" -InnoMode -LogFile "' + Log + '"' + FPurge;
  // A state file from an earlier uninstall says nothing about this one.
  DeleteFile(UninstallStatePath);
  Ran := RunPs(ExpandConstant('{app}\installer\uninstall-elevated.ps1'), Args, False, Code);
  if Ran and (Code = 3) then
  begin
    // Exit 3: the elevated work finished, except that an OpenRGB was still running, so the
    // folder it holds was kept whole instead of being deleted half way (#60).
    if not UninstallSilent then
      MsgBox('canvas-rgb is being removed, except one thing: an OpenRGB is still running, so its folder was kept whole and not deleted. Close OpenRGB (or restart the PC) and remove that folder by hand.' + #13#10 + #13#10 +
        LogTail(Log), mbInformation, MB_OK);
  end
  else if Ran and (Code = 2) then
  begin
    // Exit 2: the elevated work finished, except the SignalRGB service start type (its restore record is unreadable and kept).
    // The service is already gone, so the uninstall carries on; say what was not restored.
    if not UninstallSilent then
      MsgBox('canvas-rgb is being removed, except one thing: SignalRGB''s start-up setting was not put back (its saved record is unreadable, so it was kept).' + #13#10 + #13#10 +
        LogTail(Log), mbInformation, MB_OK);
  end
  else if (not Ran) or (Code <> 0) then
  begin
    // The uninstall can fail half way: the service or the files can already be gone (#64). The
    // script writes down what it removed, and that is what is said here. Only when it wrote
    // nothing (it failed before the machine home existed) and nothing was removed is the old
    // "nothing was removed" true.
    if not UninstallSilent then
    begin
      Msg := UninstallStateWords();
      if Msg = '' then
      begin
        if Code = 4 then
          Msg := 'canvas-rgb was not removed cleanly. The log below says what was done.'
        else
          Msg := 'canvas-rgb could not stop running or give the fans back to your motherboard (code ' + IntToStr(Code) + '). Nothing was removed.';
      end;
      MsgBox(Msg + #13#10 + #13#10 + LogTail(Log), mbError, MB_OK);
    end;
    Result := False;
  end;
end;

// After Inno's confirmation, before it removes files: the per-user cleanup. ExecAsOriginalUser is not
// supported at uninstall time, so this runs in the elevated uninstaller (right when the user elevated their own account).
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if CurUninstallStep <> usUninstall then Exit;
  // Nothing from the install folder runs unless the check in InitializeUninstall passed (#46).
  if not FInstallDirOk then Exit;
  if not RunPs(ExpandConstant('{app}\installer\uninstall-user.ps1'),
      '-InstallDir "' + ExpandConstant('{app}') + '"' + FPurge, False, Code) or (Code <> 0) then
    SuppressibleMsgBox('Cleaning up your account settings did not finish (code ' + IntToStr(Code) + '). The program files are still removed.',
      mbError, MB_OK, IDOK);
end;
