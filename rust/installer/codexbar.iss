#define MyAppName "AgentBar"
#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef TargetBinDir
  #define TargetBinDir "..\\..\\target\\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\\target\\installer"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "AgentBar-" + AppVersion + "-Setup"
#endif
#ifndef WebView2BootstrapperPath
  #define WebView2BootstrapperPath "..\\target\\installer-deps\\MicrosoftEdgeWebview2Setup.exe"
#endif

[Setup]
AppId=AgentBar
AppName={#MyAppName}
AppVersion={#AppVersion}
AppVerName={#MyAppName} {#AppVersion}
AppPublisher=AgentBar (fork of Win-CodexBar)
AppPublisherURL=https://github.com/jbrito-dev/Win-CodexBar
AppSupportURL=https://github.com/jbrito-dev/Win-CodexBar/issues
AppUpdatesURL=https://github.com/jbrito-dev/Win-CodexBar/releases
DefaultDirName={localappdata}\Programs\AgentBar
DefaultGroupName=AgentBar
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
UsePreviousAppDir=yes
CloseApplications=yes
WizardStyle=modern
Compression=lzma
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile=..\icons\icon.ico
UninstallDisplayIcon={app}\codexbar.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#TargetBinDir}\codexbar.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#TargetBinDir}\codexbar-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#TargetBinDir}\codexbar-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\icons\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#WebView2BootstrapperPath}"; Flags: dontcopy

[Icons]
Name: "{autoprograms}\AgentBar"; Filename: "{app}\codexbar.exe"; Parameters: "menubar"; WorkingDir: "{app}"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\AgentBar"; Filename: "{app}\codexbar.exe"; Parameters: "menubar"; WorkingDir: "{app}"; Tasks: desktopicon; IconFilename: "{app}\icon.ico"

[Run]
; Interactive installs: optional checkbox on the finish page.
Filename: "{app}\codexbar.exe"; Parameters: "menubar"; Description: "Launch AgentBar"; Flags: nowait postinstall skipifsilent; Check: CanLaunchCodexBar
; Silent upgrades (winget / in-app updater): always relaunch so the tray icon
; returns after CloseApplications kills the previous process. Single-instance
; handles a second launch from the updater helper if both fire.
Filename: "{app}\codexbar.exe"; Parameters: "menubar"; Flags: nowait postinstall skipifnotsilent; Check: CanLaunchCodexBar

[Code]
var
  NeedsWebView2Restart: Boolean;

function WebView2InstalledInView(RootKey: Integer): Boolean;
var
  RuntimeVersion: String;
begin
  Result :=
    RegQueryStringValue(
      RootKey,
      'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
      'pv',
      RuntimeVersion
    ) and
    (RuntimeVersion <> '');
end;

function WebView2NeedsInstall(): Boolean;
begin
  Result :=
    not WebView2InstalledInView(HKLM64) and
    not WebView2InstalledInView(HKLM32) and
    not WebView2InstalledInView(HKCU);
end;

procedure EnsureWebView2Installed();
var
  ResultCode: Integer;
begin
  if not WebView2NeedsInstall() then
    exit;

  ExtractTemporaryFile('MicrosoftEdgeWebview2Setup.exe');

  WizardForm.StatusLabel.Caption := 'Installing Microsoft Edge WebView2 Runtime...';
  WizardForm.ProgressGauge.Style := npbstMarquee;
  try
    if not Exec(
      ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe'),
      '/silent /install',
      '',
      SW_HIDE,
      ewWaitUntilTerminated,
      ResultCode
    ) then
      RaiseException('Failed to start the Microsoft Edge WebView2 Runtime installer.');

    if (ResultCode <> 0) and (ResultCode <> 1638) and (ResultCode <> 3010) then
      RaiseException(
        'Microsoft Edge WebView2 Runtime installation failed with exit code ' +
        IntToStr(ResultCode) +
        '.'
      );

    if ResultCode = 3010 then
      NeedsWebView2Restart := True;
  finally
    WizardForm.ProgressGauge.Style := npbstNormal;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then begin
    EnsureWebView2Installed();
  end;
end;

function NeedRestart(): Boolean;
begin
  Result := NeedsWebView2Restart;
end;

function CanLaunchCodexBar(): Boolean;
begin
  Result := not NeedsWebView2Restart;
end;
