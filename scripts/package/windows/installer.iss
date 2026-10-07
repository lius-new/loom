; Loom per-user installer, modelled on Zed's Inno Setup installer
; (crates/zed/resources/windows/zed.iss in zed-industries/zed).
;
; Built by `node scripts/package/package.mjs bundle`, which passes:
;   Version, VersionQuad, StageDir, OutputDir, OutputBase, AppIcon, Homepage
;
; Installs into %LOCALAPPDATA%\Programs\Loom without elevation. User data
; (sessions, settings) is never removed.

#define AppName "Loom"
#define AppExeName "Loom.exe"

[Setup]
; Never change the AppId: it identifies the installation across upgrades.
AppId={{E00707BB-1779-4DA4-ADCB-7604AB62244A}
AppName={#AppName}
AppVersion={#Version}
AppVerName={#AppName} {#Version}
AppPublisher={#AppName}
AppPublisherURL={#Homepage}
AppSupportURL={#Homepage}
AppUpdatesURL={#Homepage}
VersionInfoVersion={#VersionQuad}
VersionInfoProductName={#AppName}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
SetupArchitecture=x64
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
WizardStyle=modern dynamic
ShowLanguageDialog=auto
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBase}
Compression=lzma2
SolidCompression=yes
; Offer to close a running Loom before its files are replaced.
CloseApplications=yes
RestartApplications=no
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[CustomMessages]
english.StartMenuIcon=Create a Start menu shortcut
chinesesimplified.StartMenuIcon=创建开始菜单快捷方式
english.Other=Other:
chinesesimplified.Other=其他:
english.AddContextMenuFiles=Add "Open with %1" action to Windows Explorer file context menu
chinesesimplified.AddContextMenuFiles=将“通过 %1 打开”操作添加到 Windows 资源管理器文件上下文菜单
english.AddContextMenuFolders=Add "Open with %1" action to Windows Explorer directory context menu
chinesesimplified.AddContextMenuFolders=将“通过 %1 打开”操作添加到 Windows 资源管理器目录上下文菜单
english.AddToPath=Add to PATH (requires shell restart)
chinesesimplified.AddToPath=添加到 PATH (重启终端后生效)
english.OpenWith=Open with %1
chinesesimplified.OpenWith=通过 %1 打开

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "startmenuicon"; Description: "{cm:StartMenuIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "addcontextmenufiles"; Description: "{cm:AddContextMenuFiles,{#AppName}}"; GroupDescription: "{cm:Other}"
Name: "addcontextmenufolders"; Description: "{cm:AddContextMenuFolders,{#AppName}}"; GroupDescription: "{cm:Other}"
Name: "addtopath"; Description: "{cm:AddToPath}"; GroupDescription: "{cm:Other}"

[InstallDelete]
; Replace the managed Git runtime wholesale so files dropped by a newer
; MinGit never linger next to the new ones.
Type: filesandordirs; Name: "{app}\runtime"
; Shortcuts follow this run's choices on upgrade.
Type: files; Name: "{autodesktop}\{#AppName}.lnk"; Tasks: not desktopicon
Type: files; Name: "{autoprograms}\{#AppName}.lnk"; Tasks: not startmenuicon

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: startmenuicon
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
; "Open with Loom" for files.
Root: HKCU; Subkey: "Software\Classes\*\shell\{#AppName}"; ValueType: none; Flags: deletekey; Tasks: not addcontextmenufiles
Root: HKCU; Subkey: "Software\Classes\*\shell\{#AppName}"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWith,{#AppName}}"; Flags: uninsdeletekey; Tasks: addcontextmenufiles
Root: HKCU; Subkey: "Software\Classes\*\shell\{#AppName}"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExeName}"; Tasks: addcontextmenufiles
Root: HKCU; Subkey: "Software\Classes\*\shell\{#AppName}\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""; Tasks: addcontextmenufiles

; "Open with Loom" for folders, folder backgrounds and drives.
Root: HKCU; Subkey: "Software\Classes\Directory\shell\{#AppName}"; ValueType: none; Flags: deletekey; Tasks: not addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\shell\{#AppName}"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWith,{#AppName}}"; Flags: uninsdeletekey; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\shell\{#AppName}"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExeName}"; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\shell\{#AppName}\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\{#AppName}"; ValueType: none; Flags: deletekey; Tasks: not addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\{#AppName}"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWith,{#AppName}}"; Flags: uninsdeletekey; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\{#AppName}"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExeName}"; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\{#AppName}\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%V"""; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Drive\shell\{#AppName}"; ValueType: none; Flags: deletekey; Tasks: not addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Drive\shell\{#AppName}"; ValueType: string; ValueName: ""; ValueData: "{cm:OpenWith,{#AppName}}"; Flags: uninsdeletekey; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Drive\shell\{#AppName}"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExeName}"; Tasks: addcontextmenufolders
Root: HKCU; Subkey: "Software\Classes\Drive\shell\{#AppName}\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%V"""; Tasks: addcontextmenufolders

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvironmentKey = 'Environment';

function BinDir: String;
begin
  Result := ExpandConstant('{app}\bin');
end;

function SamePathEntry(Entry, Dir: String): Boolean;
begin
  Result := CompareText(RemoveBackslashUnlessRoot(Trim(Entry)), RemoveBackslashUnlessRoot(Dir)) = 0;
end;

{ Return Path without any entry equal to Dir; every other entry is kept as is. }
function WithoutPathEntry(Path, Dir: String): String;
var
  Rest, Entry: String;
  Separator: Integer;
begin
  Result := '';
  Rest := Path;
  while Rest <> '' do
  begin
    Separator := Pos(';', Rest);
    if Separator = 0 then
    begin
      Entry := Rest;
      Rest := '';
    end
    else
    begin
      Entry := Copy(Rest, 1, Separator - 1);
      Delete(Rest, 1, Separator);
    end;
    if not SamePathEntry(Entry, Dir) then
    begin
      if Result <> '' then
        Result := Result + ';';
      Result := Result + Entry;
    end;
  end;
end;

{ Add or remove Dir in the user's PATH, leaving it untouched when nothing changes. }
procedure UpdateUserPath(Dir: String; Add: Boolean);
var
  OldPath, NewPath: String;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', OldPath) then
    OldPath := '';
  NewPath := WithoutPathEntry(OldPath, Dir);
  if Add then
  begin
    if (NewPath <> '') and (NewPath[Length(NewPath)] <> ';') then
      NewPath := NewPath + ';';
    NewPath := NewPath + Dir;
  end;
  if NewPath <> OldPath then
    RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', NewPath);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    UpdateUserPath(BinDir, WizardIsTaskSelected('addtopath'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    UpdateUserPath(BinDir, False);
end;
