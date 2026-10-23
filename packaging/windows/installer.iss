; Inno Setup script for Scratchpad. Built by scripts/package-windows.ps1.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\scratchpad.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{B8A190E9-47AF-4893-83B9-42C9566E276D}
AppName=Scratchpad
AppVersion={#AppVersion}
AppPublisher=ScaleXY
AppPublisherURL=https://github.com/scalexy/scratchpad
DefaultDirName={autopf}\ScaleXY\Scratchpad
DefaultGroupName=Scratchpad
UninstallDisplayIcon={app}\scratchpad.exe
OutputDir={#OutputDir}
OutputBaseFilename=scratchpad-{#AppVersion}-windows-{#Arch}-setup
SetupIconFile=..\icons\scratchpad.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequiredOverridesAllowed=dialog
#if Arch == "x64"
ArchitecturesAllowed=x64compatible and not arm64
ArchitecturesInstallIn64BitMode=x64compatible
#elif Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x86compatible
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "scratchpad.exe"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

[Icons]
Name: "{group}\Scratchpad"; Filename: "{app}\scratchpad.exe"
Name: "{group}\Scratchpad (headless server)"; Filename: "{app}\scratchpad.exe"; Parameters: "--headless"
Name: "{group}\Uninstall Scratchpad"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Scratchpad"; Filename: "{app}\scratchpad.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\scratchpad.exe"; Description: "{cm:LaunchProgram,Scratchpad}"; Flags: nowait postinstall skipifsilent
