; DPI UI - Inno Setup kurulum betigi
;
; Derleme: powershell -File publish.ps1   (once dosyalari build\app'e toplar, sonra ISCC calistirir)
; Cikti:   build\DPI-UI-Setup.exe
;
; Kurulum yonetici ister: motor bir Windows hizmetidir ve WinDivert surucusunu yukler.
; Arayuz ise kurulumdan sonra normal kullanici olarak acilir (runasoriginaluser);
; gunluk kullanimda UAC hic sorulmaz.
;
; Guncellemeler: motor hizmeti (SYSTEM) paketi /VERYSILENT /UPDATE=service ile kendisi kurar;
; kullaniciya hicbir sey sorulmaz. Bu kipte kurulum arayuzu acmaz (SYSTEM olarak acardi);
; kurulumdan sonra baslayan yeni hizmet arayuzu kullanicinin oturumunda yeniden acar.
; /UPDATE=window|tray: 3.0.3 ve oncesinin arayuzden UAC ile baslattigi eski yol.

#define AppName "DPI UI"
#define AppVersion "3.0.5"
#define AppPublisher "DPI UI"
#define AppExe "GoodbyeDPI3.exe"
#define ServiceExe "GoodbyeDPI3-Service.exe"
#define ServiceName "GoodbyeDPI3"
#define SourceDir "..\build\app"
#define V1AppId "{9DCDD6ED-2D8D-4EEE-A4A4-13F6B37614AE}_is1"

[Setup]
AppId={{5B7C2E41-9A3D-4F6E-B8C1-7D2A0E9F4C63}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppSupportURL=https://github.com/unsalable/dpi-ui
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir=..\build
OutputBaseFilename=DPI-UI-Setup
SetupIconFile=..\build\app.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
WizardSizePercent=100
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
CloseApplications=yes
; Pencere ve tepsi ayni guncellemeyi baslatirsa ikinci kurulum calismaz.
SetupMutex=GoodbyeDPI3Setup
RestartApplications=no
VersionInfoVersion=3.0.0.0
VersionInfoProductName={#AppName}

[Languages]
Name: "tr"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
tr.V1Found=GoodbyeDPI UI'ın eski sürümü (2.x) bu bilgisayarda kurulu.%n%nİki DPI aracı aynı anda çalışırsa bağlantılar bozulabilir. Eski sürüm şimdi kaldırılsın mı?%n%nAyarların (sağlayıcı, özel yöntemler, özel DNS) yeni sürüme otomatik aktarılır.
en.V1Found=An older GoodbyeDPI UI (2.x) is installed.%n%nRunning two DPI tools at once can break connections. Uninstall the old version now?%n%nYour settings will be imported into the new version.
tr.Launch=DPI UI uygulamasını aç
en.Launch=Open DPI UI

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\{#ServiceExe}"; DestDir: "{app}"; Flags: ignoreversion
; WinDivert: surum karsilastirilir (ayni surum yeniden yazilmaz). Surucu cekirdekte yuklu kaldigi
; icin dosya kilitli olabilir; o durumda hata yerine yeniden baslatmada degistirilir.
Source: "{#SourceDir}\WinDivert.dll"; DestDir: "{app}"; Flags: restartreplace uninsrestartdelete
Source: "{#SourceDir}\WinDivert64.sys"; DestDir: "{app}"; Flags: restartreplace uninsrestartdelete
Source: "{#SourceDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[InstallDelete]
; 3.0.0 oncesi ad (GoodbyeDPI UI 3) ile olusturulmus kisayollar.
Type: filesandordirs; Name: "{commonprograms}\GoodbyeDPI UI 3"
Type: files; Name: "{commondesktop}\GoodbyeDPI UI 3.lnk"
Type: files; Name: "{userdesktop}\GoodbyeDPI UI 3.lnk"

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Oturum acilisi kaydini arayuz yazar; kaldirirken temizle (deger burada olusturulmaz).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "GoodbyeDPI3"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\{#ServiceExe}"; Parameters: "--install"; Flags: runhidden waituntilterminated; StatusMsg: "Motor hizmeti kuruluyor..."
Filename: "{sys}\net.exe"; Parameters: "start {#ServiceName}"; Flags: runhidden waituntilterminated; StatusMsg: "Motor başlatılıyor..."
Filename: "{app}\{#AppExe}"; Description: "{cm:Launch}"; Flags: postinstall nowait skipifsilent runasoriginaluser
; Elle kurulumda tepsi her durumda baslar: uygulama calisir ve oturum acilisina kaydolur.
Filename: "{app}\{#AppExe}"; Parameters: "--tray"; Flags: nowait skipifsilent runasoriginaluser
; Uygulamanin baslattigi guncelleme (/UPDATE=window|tray): bitince kullanicinin kendi yetkisiyle yeniden acilir.
Filename: "{app}\{#AppExe}"; Flags: nowait runasoriginaluser; Check: IsUpdate('window')
Filename: "{app}\{#AppExe}"; Parameters: "--tray"; Flags: nowait runasoriginaluser; Check: IsUpdate('tray')

[UninstallRun]
; Once duzgun kapat (tepsi simgesi temizlenir), kalan olursa zorla kapat.
Filename: "{app}\{#AppExe}"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitUi"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM {#AppExe}"; Flags: runhidden waituntilterminated; RunOnceId: "KillUi"
Filename: "{app}\{#ServiceExe}"; Parameters: "--uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"

[UninstallDelete]
; Ayarlar (%ProgramData%\GoodbyeDPI3) bilerek birakilir; yeniden kurulumda geri gelir.
Type: filesandordirs; Name: "{app}\licenses"

[Code]
var
  // PrepareToInstall motor hizmetini durdurdu mu? Kurulum yarida kalirsa hizmet yeniden baslatilir.
  StoppedService: Boolean;

function V1UninstallString(): String;
var
  S: String;
begin
  Result := '';
  if RegQueryStringValue(HKLM64, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#V1AppId}', 'UninstallString', S) then
    Result := S
  else if RegQueryStringValue(HKLM32, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#V1AppId}', 'UninstallString', S) then
    Result := S;
end;

function IsUpdate(Kind: String): Boolean;
begin
  Result := CompareText(ExpandConstant('{param:UPDATE|}'), Kind) = 0;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Uninst: String;
begin
  Result := '';
  // Guncelleme: dosyalar kilitli olmasin diye once hizmet ve arayuz kapatilir.
  Exec(ExpandConstant('{sys}\net.exe'), 'stop {#ServiceName}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  StoppedService := True;
  // Once duzgun kapat (tepsi simgesi temizlenir). Beklenmez: --quit bilmeyen eski bir surum
  // normal acilirsa kurulum takilmasin; kalan her sey asagida zorla kapatilir.
  if FileExists(ExpandConstant('{app}\{#AppExe}')) then
  begin
    Exec(ExpandConstant('{app}\{#AppExe}'), '--quit', '', SW_HIDE, ewNoWait, Code);
    Sleep(500);
  end;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);

  Uninst := V1UninstallString();
  if (Uninst <> '') and (not WizardSilent) then
  begin
    if MsgBox(CustomMessage('V1Found'), mbConfirmation, MB_YESNO) = IDYES then
    begin
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM GoodbyeDPI-UI.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
      Exec(RemoveQuotes(Uninst), '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, Code);
    end;
  end;

  // WinDivert surucusu son tutamac kapandiktan sonra da cekirdekte yuklu kalir ve
  // WinDivert64.sys dosyasini kilitler. Baska bir arac kullanmiyorsa burada durur;
  // kullaniyorsa durmaz, dosya o zaman ayni surumse atlanir, degilse yeniden baslatmada degisir.
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop WinDivert', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(300);
end;

procedure DeinitializeSetup();
var
  Code: Integer;
begin
  // Kurulum basarili olduysa hizmet [Run] ile zaten basladi ("zaten calisiyor" hatasi zararsiz).
  // Yarida kaldiysa eski hizmet yeniden baslar: baglanti korumasiz kalmaz, eski hizmet de
  // kapanan arayuzu yeniden acar.
  if StoppedService then
    Exec(ExpandConstant('{sys}\net.exe'), 'start {#ServiceName}', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
