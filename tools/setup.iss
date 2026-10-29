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
english.AssocTask=Register qview for image files (JPEG, PNG, GIF, WebP, BMP, TIFF, SVG, CBZ and CBR comic books and others)
russian.AssocTask=Зарегистрировать qview для файлов изображений (JPEG, PNG, GIF, WebP, BMP, TIFF, SVG, комиксов CBZ и CBR и других)
english.AssocStatus=Registering file types...
russian.AssocStatus=Регистрация типов файлов...
english.ChooseDefault=Choose qview as the default image viewer (opens Windows Settings)
russian.ChooseDefault=Выбрать qview программой просмотра по умолчанию (откроются параметры Windows)
english.FolderMenuGroup=Explorer:
russian.FolderMenuGroup=Проводник:
english.FolderMenuTask=Add "Browse in qview" to the context menu of folders and drives
russian.FolderMenuTask=Добавить «Просмотреть в qview» в контекстное меню папок и дисков
english.FolderMenuItem=Browse in qview
russian.FolderMenuItem=Просмотреть в qview

[Tasks]
Name: "assoc"; Description: "{cm:AssocTask}"; GroupDescription: "{cm:AssocGroup}"
Name: "foldermenu"; Description: "{cm:FolderMenuTask}"; GroupDescription: "{cm:FolderMenuGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Registry]
; "Browse in qview" in the context menu of a folder, of a folder's empty
; space and of a drive: the folder opens in the gallery (per user, under
; HKCU, like the file types; removed when the task is cleared on a later
; installation and on uninstallation). %1 is the folder clicked, %V the
; folder whose empty space was.
Root: HKCU; Subkey: "Software\Classes\Directory\shell\qview"; ValueType: string; ValueName: ""; ValueData: "{cm:FolderMenuItem}"; Flags: uninsdeletekey; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\qview"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\qview\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\qview"; ValueType: string; ValueName: ""; ValueData: "{cm:FolderMenuItem}"; Flags: uninsdeletekey; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\qview"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\qview\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%V"""; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\qview"; ValueType: string; ValueName: ""; ValueData: "{cm:FolderMenuItem}"; Flags: uninsdeletekey; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\qview"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\qview\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\qview"; Flags: deletekey; Tasks: not foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\qview"; Flags: deletekey; Tasks: not foldermenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\qview"; Flags: deletekey; Tasks: not foldermenu

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; libheif with libde265 and dav1d for HEIC/HEIF and AVIF (tools\build_heif.py),
; LGPL: separate DLLs (dav1d, BSD, linked into heif.dll); their licences and
; sources in THIRD_PARTY_LICENSES.md and licenses.
Source: "..\target\release\heif.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\libde265.dll"; DestDir: "{app}"; Flags: ignoreversion
; The licences (make_release.py's LICENSE_FILES; the exe shows them too).
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\licenses\LGPL-3.0.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\licenses\dav1d-license.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\licenses\unrar-license.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\licenses\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.ru.md"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; The licences folder of earlier versions, written by build_heif.py.
Type: files; Name: "{app}\licenses\README.txt"
Type: files; Name: "{app}\licenses\libheif-COPYING.txt"
Type: files; Name: "{app}\licenses\libde265-COPYING.txt"

[UninstallDelete]
; qview's own thumbnail cache (src\thumbcache.rs); the settings stay.
Type: filesandordirs; Name: "{localappdata}\qview\thumbs"

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
