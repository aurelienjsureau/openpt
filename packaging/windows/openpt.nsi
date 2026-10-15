Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!define ROOT "..\.."
!define APP "OpenPT"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

Name "${APP} ${VERSION}"
OutFile "${OUT}"
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "Software\${APP}" "InstallDir"
RequestExecutionLevel admin
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "CompanyName" "BourrinAudio"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 BourrinAudio and the OpenPT contributors"

!include "MUI2.nsh"
!include "x64.nsh"

!define MUI_ICON "${ROOT}\assets\app-icon\openpt.ico"
!define MUI_UNICON "${ROOT}\assets\app-icon\openpt.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\openpt.exe"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${ROOT}\LICENSE-MIT"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${APP} nécessite Windows 64 bits."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "OpenPT" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${BIN}\openpt.exe"
  File "${VCRT}\vcruntime140.dll"
  File "${VCRT}\vcruntime140_1.dll"
  File "${ROOT}\LICENSE-MIT"
  File "${ROOT}\LICENSE-APACHE"
  File "${ROOT}\NOTICE"
  File "${ROOT}\ATTRIBUTION.md"
  File "${ROOT}\assets\app-icon\openpt.ico"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\openpt.exe" "" "$INSTDIR\openpt.exe" 0

  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\App Paths\openpt.exe" "" "$INSTDIR\openpt.exe"

  WriteRegStr HKLM "Software\Classes\.openpt" "" "OpenPT.Session"
  WriteRegStr HKLM "Software\Classes\.openpt" "Content Type" "application/x-openpt-session"
  WriteRegStr HKLM "Software\Classes\OpenPT.Session" "" "OpenPT Session"
  WriteRegStr HKLM "Software\Classes\OpenPT.Session\DefaultIcon" "" "$INSTDIR\openpt.exe,0"
  WriteRegStr HKLM "Software\Classes\OpenPT.Session\shell\open\command" "" '"$INSTDIR\openpt.exe" "%1"'
  WriteRegStr HKLM "Software\Classes\OpenPT.ProToolsSession" "" "Pro Tools Session"
  WriteRegStr HKLM "Software\Classes\OpenPT.ProToolsSession\DefaultIcon" "" "$INSTDIR\openpt.exe,0"
  WriteRegStr HKLM "Software\Classes\OpenPT.ProToolsSession\shell\open\command" "" '"$INSTDIR\openpt.exe" "%1"'
  WriteRegStr HKLM "Software\Classes\.ptx\OpenWithProgids" "OpenPT.ProToolsSession" ""
  WriteRegStr HKLM "Software\Classes\Applications\openpt.exe" "FriendlyAppName" "${APP}"
  WriteRegStr HKLM "Software\Classes\Applications\openpt.exe\shell\open\command" "" '"$INSTDIR\openpt.exe" "%1"'

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "BourrinAudio"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\openpt.exe,0"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section "Raccourci sur le bureau" SecDesktop
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\openpt.exe" "" "$INSTDIR\openpt.exe" 0
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\openpt.exe"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\NOTICE"
  Delete "$INSTDIR\ATTRIBUTION.md"
  Delete "$INSTDIR\openpt.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"

  ReadRegStr $0 HKLM "Software\Classes\.openpt" ""
  StrCmp $0 "OpenPT.Session" 0 +2
    DeleteRegKey HKLM "Software\Classes\.openpt"
  DeleteRegKey HKLM "Software\Classes\OpenPT.Session"
  DeleteRegKey HKLM "Software\Classes\OpenPT.ProToolsSession"
  DeleteRegValue HKLM "Software\Classes\.ptx\OpenWithProgids" "OpenPT.ProToolsSession"
  DeleteRegKey HKLM "Software\Classes\Applications\openpt.exe"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\App Paths\openpt.exe"
  DeleteRegKey HKLM "Software\${APP}"
  DeleteRegKey HKLM "${UNINST_KEY}"

  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd
