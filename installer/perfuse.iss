#ifndef PerfuseExe
  #define PerfuseExe "..\target\release\perfuse.exe"
#endif
#ifndef PerfuseVersion
  #error Supply /DPerfuseVersion=<version> to ISCC
#endif

[Setup]
AppId={{70FD522D-632C-430E-B79E-673ED27CD092}
AppName=Perfuse
AppVersion={#PerfuseVersion}
AppPublisher=Perfuse
DefaultDirName={localappdata}\Programs\Perfuse
DefaultGroupName=Perfuse
DisableDirPage=yes
DisableProgramGroupPage=yes
UsePreviousAppDir=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
OutputDir=..\dist
OutputBaseFilename=Perfuse-v{#PerfuseVersion}-windows-x64-setup
UninstallDisplayIcon={app}\perfuse.exe
CloseApplications=yes
RestartApplications=no
WizardStyle=modern

[Files]
Source: "{#PerfuseExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Perfuse"; Filename: "{app}\perfuse.exe"

[Run]
Filename: "{app}\perfuse.exe"; Description: "Launch Perfuse"; Flags: nowait postinstall skipifsilent
