Unicode True
!include "MUI2.nsh"
!include "FileFunc.nsh"
!ifndef APPVERSION
!define APPVERSION "0.1.1"
!endif
!ifndef PRODUCTVERSION
!define PRODUCTVERSION "${APPVERSION}.0"
!endif

Name "DeezNats"
OutFile "${OUTPUT}\DeezNats-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\DeezNats"
InstallDirRegKey HKCU "Software\DeezNats" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
Icon "${PROJECT}\assets\DeezNats.ico"
UninstallIcon "${PROJECT}\assets\DeezNats.ico"
VIProductVersion "${PRODUCTVERSION}"
VIAddVersionKey "ProductName" "DeezNats"
VIAddVersionKey "FileDescription" "DeezNats per-user installer"
VIAddVersionKey "FileVersion" "${APPVERSION}"
VIAddVersionKey "LegalCopyright" "DeezNats contributors; AGPL-3.0"
Var TestMode

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PROJECT}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  StrCpy $TestMode ""
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/TESTMODE" $1
  IfErrors +2
    StrCpy $TestMode "1"
FunctionEnd

Section "DeezNats"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  ; This installer uses no MSI, restore-point API, service, or restore plugin.
  File "${PAYLOAD}\deeznats-prototype.exe"
  File "${PAYLOAD}\libdeezercore.dll"
  File "${PROJECT}\assets\DeezNats.ico"
  File "${PROJECT}\LICENSE"
  File "${PROJECT}\NOTICE.md"
  File /oname=qbsdiff.LICENSE "${PROJECT}\assets\licenses\qbsdiff.txt"
  File "${PROJECT}\SOURCE-AND-DEPENDENCIES.md"
  File "${OUTPUT}\DeezNats-Source.zip"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  StrCmp $TestMode "1" test_install
  WriteRegStr HKCU "Software\DeezNats" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "DisplayName" "DeezNats"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "DisplayVersion" "${APPVERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "DisplayIcon" "$INSTDIR\DeezNats.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats" "NoRepair" 1
  CreateShortcut "$SMPROGRAMS\DeezNats.lnk" "$INSTDIR\deeznats-prototype.exe" "" "$INSTDIR\DeezNats.ico"
  Goto install_done
  test_install:
  FileOpen $0 "$INSTDIR\.installer-test" w
  FileClose $0
  install_done:
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\.installer-test" skip_registration
  ; The app owns the Soloist process and terminates it when it exits.
  nsExec::ExecToStack 'taskkill /IM deeznats-prototype.exe /T /F'
  Pop $0
  Pop $1
  InitPluginsDir
  File /oname=$PLUGINSDIR\Remove-UserData.ps1 "${PROJECT}\installer\Remove-UserData.ps1"
  nsExec::ExecToStack 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\Remove-UserData.ps1"'
  Pop $0
  Pop $1
  StrCmp $0 "0" +3
  MessageBox MB_ICONSTOP "Could not remove DeezNats user data. Close DeezNats, reconnect any data drive, and retry.$\r$\n$1"
  Abort
  Delete "$SMPROGRAMS\DeezNats.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeezNats"
  DeleteRegKey HKCU "Software\DeezNats"
  skip_registration:
  ; Delete installed files while preserving unknown files in the install directory.
  Delete "$INSTDIR\deeznats-prototype.exe"
  Delete "$INSTDIR\libdeezercore.dll"
  Delete "$INSTDIR\DeezNats.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE.md"
  Delete "$INSTDIR\qbsdiff.LICENSE"
  Delete "$INSTDIR\SOURCE-AND-DEPENDENCIES.md"
  Delete "$INSTDIR\DeezNats-Source.zip"
  Delete "$INSTDIR\.installer-test"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
