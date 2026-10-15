; Null's Windows installer (Inno Setup 6): for the person installing, no administrator
; needed, into %LOCALAPPDATA%\Programs\Null; a Start menu entry, `null` for terminals (the
; folder on the PATH), "Open with Null" on folders and files, and an uninstaller in
; Settings > Apps (which asks whether the settings, keys and data go too). Built by package.ps1:
;
;     iscc /DVersion=0.1.1 packaging\windows\null.iss

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{4F1B7C2E-9D3A-4E58-B6C1-2A7E0D9F5B13}
AppName=Null
AppVersion={#Version}
AppVerName=Null {#Version}
AppPublisher=ShortPlanet3058
AppPublisherURL=https://null-ide.com/
AppSupportURL=https://github.com/ShortPlanet3058/null-ide/issues
AppUpdatesURL=https://github.com/ShortPlanet3058/null-ide/releases
DefaultDirName={localappdata}\Programs\Null
DisableProgramGroupPage=yes
DisableDirPage=auto
; (Installing again after an uninstall: no "the folder exists" question.)
DirExistsWarning=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
ChangesAssociations=yes
LicenseFile=..\..\LICENSE
SetupIconFile=null.ico
UninstallDisplayIcon={app}\null.exe
UninstallDisplayName=Null
OutputDir=..\..\target\release
OutputBaseFilename=null-{#Version}-windows-x86_64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[CustomMessages]
en.AddToPath=Add "null" to the PATH, to open folders from a terminal (null .)
fr.AddToPath=Ajouter « null » au PATH, pour ouvrir des dossiers depuis un terminal (null .)
en.OpenWith=Add "Open with Null" to folders and files
fr.OpenWith=Ajouter « Ouvrir avec Null » aux dossiers et fichiers
en.OpenWithNull=Open with Null
fr.OpenWithNull=Ouvrir avec Null
en.StartNull=Start Null
fr.StartNull=Démarrer Null
en.KeepQuestion=Keep your settings?
fr.KeepQuestion=Garder vos réglages ?
en.KeepDetail=Your settings, saved API keys, history and downloaded language servers, for if you install Null again. Or delete them now.
fr.KeepDetail=Vos réglages, clés d'API enregistrées, historique et serveurs de langage téléchargés, si vous réinstallez Null. Ou supprimez-les maintenant.
en.DeleteButton=Delete them
fr.DeleteButton=Supprimer
en.KeepButton=Keep them
fr.KeepButton=Garder

[Tasks]
Name: "path"; Description: "{cm:AddToPath}"
Name: "openwith"; Description: "{cm:OpenWith}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\null.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Null"; Filename: "{app}\null.exe"
Name: "{autodesktop}\Null"; Filename: "{app}\null.exe"; Tasks: desktopicon

[Registry]
; "Open with Null" on a folder, inside a folder (its background) and on any file.
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Null"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWithNull}"; Tasks: openwith; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Null"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\null.exe"""; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Null\command"; ValueType: string; ValueName: ""; ValueData: """{app}\null.exe"" ""%V"""; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Null"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWithNull}"; Tasks: openwith; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Null"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\null.exe"""; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Null\command"; ValueType: string; ValueName: ""; ValueData: """{app}\null.exe"" ""%V"""; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\*\shell\Null"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWithNull}"; Tasks: openwith; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\Null"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\null.exe"""; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\*\shell\Null\command"; ValueType: string; ValueName: ""; ValueData: """{app}\null.exe"" ""%1"""; Tasks: openwith

[Run]
Filename: "{app}\null.exe"; Description: "{cm:StartNull}"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvKey = 'Environment';

{ The folder on the person's PATH (once), and off it again when Null is uninstalled. }
function OnPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then
    Path := '';
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') > 0;
end;

procedure AddToPath(Dir: string);
var
  Path: string;
begin
  if OnPath(Dir) then
    exit;
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then
    Path := '';
  if (Path <> '') and (Copy(Path, Length(Path), 1) <> ';') then
    Path := Path + ';';
  RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path + Dir);
end;

procedure RemoveFromPath(Dir: string);
var
  Path: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then
    exit;
  Path := ';' + Path + ';';
  P := Pos(';' + Uppercase(Dir) + ';', Uppercase(Path));
  if P = 0 then
    exit;
  Delete(Path, P, Length(Dir) + 1);
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('path') then
    AddToPath(ExpandConstant('{app}'));
end;

{ Asked before anything is removed (null.exe still there to forget the keys); a silent
  uninstall keeps everything. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if (CurUninstallStep = usUninstall) and not UninstallSilent then
    { Keep first: Enter keeps (a MsgBox ignores MB_DEFBUTTON2, and the task dialog's
      Default is only what a silenced one answers). }
    if SuppressibleTaskDialogMsgBox(CustomMessage('KeepQuestion'), CustomMessage('KeepDetail'), mbConfirmation,
      MB_YESNO, [CustomMessage('KeepButton'), CustomMessage('DeleteButton')], 0, IDYES) = IDNO then
    begin
      Exec(ExpandConstant('{app}\null.exe'), '--forget-keys', '', SW_HIDE, ewWaitUntilTerminated, Code);
      DelTree(ExpandConstant('{userappdata}\Null'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\Null'), True, True, True);
    end;
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
