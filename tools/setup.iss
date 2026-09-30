; qview installer (Inno Setup 6).
; Built by tools\make_release.py, which passes the version and the icon path:
;   ISCC /DMyAppVersion=0.1.0 /DVersionInfoVersion=0.1.0.0 /DAppIcon=..\target\app.ico tools\setup.iss

#define MyAppName "qview"
#define MyAppExeName "qview.exe"
#define MyAppPublisher "Fan4_Metal"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef VersionInfoVersion
  #define VersionInfoVersion "0.0.0.0"
#endif
#ifndef AppIcon
  #define AppIcon "..\target\app.ico"
#endif

[Setup]
AppId={{A06A0881-69DE-4D38-86C4-EC1FC1D9D86C}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
VersionInfoVersion={#VersionInfoVersion}
AppCopyright=Copyright (C) 2026 {#MyAppPublisher}
AppPublisher={#MyAppPublisher}
; Per-user installation without administrator rights:
; {autopf} points to %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=qview_{#MyAppVersion}_Setup
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\{#MyAppExeName}
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; File associations change: Explorer is told to refresh its icons.
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.AssocGroup=File types:
russian.AssocGroup=Типы файлов:
english.AssocTask=Register qview for image files (JPEG, PNG, GIF, WebP, BMP, TIFF and others)
russian.AssocTask=Зарегистрировать qview для файлов изображений (JPEG, PNG, GIF, WebP, BMP, TIFF и других)
english.AssocStatus=Registering file types...
russian.AssocStatus=Регистрация типов файлов...
english.ChooseDefault=Choose qview as the default image viewer (opens Windows Settings)
russian.ChooseDefault=Выбрать qview программой просмотра по умолчанию (откроются параметры Windows)

[Tasks]
Name: "assoc"; Description: "{cm:AssocTask}"; GroupDescription: "{cm:AssocGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.ru.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
; The file types are registered by the program itself (src\assoc.rs, under
; HKCU): a ProgID with an icon for each format, "Open with" entries and
; Default apps capabilities. Windows lets only the user choose the default
; program, so the last page offers to open Settings for that.
Filename: "{app}\{#MyAppExeName}"; Parameters: "--register"; StatusMsg: "{cm:AssocStatus}"; Flags: runhidden waituntilterminated; Tasks: assoc
Filename: "ms-settings:defaultapps?registeredAppUser=qview"; Description: "{cm:ChooseDefault}"; Flags: shellexec nowait postinstall skipifsilent unchecked; Tasks: assoc
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Removes everything --register wrote; nothing happens if it was not run.
Filename: "{app}\{#MyAppExeName}"; Parameters: "--unregister"; RunOnceId: "UnregisterFileTypes"; Flags: runhidden waituntilterminated
