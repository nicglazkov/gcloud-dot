; GCloud Dot, per-user Windows installer.
;
; Installs into LOCALAPPDATA and writes only under HKCU, so it never asks for
; administrator rights. A tray utility that demands elevation to install is
; asking for more trust than it needs.

!include "MUI2.nsh"
!include "FileFunc.nsh"    ; ${GetSize}
!include "WordFunc.nsh"    ; ${WordFind}
!include "WinMessages.nsh" ; ${HWND_BROADCAST}, ${WM_WININICHANGE}

!ifndef VERSION
  !define VERSION "1.0.0"
!endif

Name "GCloud Dot"
OutFile "GCloud-Dot-${VERSION}-setup.exe"
Unicode True
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\GCloud Dot"
InstallDirRegKey HKCU "Software\GCloudDot" "InstallDir"
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "GCloud Dot"
VIAddVersionKey "CompanyName" "Nicholas Glazkov"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 Nicholas Glazkov. MIT licensed."
VIAddVersionKey "FileDescription" "GCloud Dot installer"
VIAddVersionKey "FileVersion" "${VERSION}"

!define MUI_ABORTWARNING
!define MUI_ICON "gcloud-dot.ico"
!define MUI_UNICON "gcloud-dot.ico"

!insertmacro MUI_PAGE_LICENSE "..\..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\gcloud-dot-tray.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start GCloud Dot now"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "GCloud Dot" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"

  ; Stop a running copy first: Windows will not overwrite a running image, and
  ; the failure it produces mid-install is unhelpfully worded.
  nsExec::ExecToLog 'taskkill /IM gcloud-dot-tray.exe /F'
  Pop $0

  File "gcloud-dot-tray.exe"
  File "gcloud-dot.exe"
  File "gcloud-dot.ico"

  WriteRegStr HKCU "Software\GCloudDot" "InstallDir" "$INSTDIR"

  ; Windows reads the name and icon for an unpackaged app's notifications from
  ; here. Without it every toast is attributed to PowerShell, which is both
  ; wrong and alarming for an app that watches credentials. The app writes
  ; these too, so an install done by the script route is covered as well.
  !define AUMID "Software\Classes\AppUserModelId\nicglazkov.GCloudDot"
  WriteRegStr HKCU "${AUMID}" "DisplayName" "GCloud Dot"
  WriteRegStr HKCU "${AUMID}" "IconUri" "$INSTDIR\gcloud-dot.ico"

  ; Put the CLI on PATH for this user. Broadcasting the change means an already
  ; open Explorer hands the new PATH to shells started afterwards.
  ReadRegStr $0 HKCU "Environment" "Path"
  ${WordFind} "$0" "$INSTDIR" "E+1{" $1
  StrCmp $1 "$0" 0 pathAlreadyPresent
    StrCmp $0 "" 0 +3
      WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR"
      Goto pathDone
    WriteRegExpandStr HKCU "Environment" "Path" "$0;$INSTDIR"
  pathDone:
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=3000
  pathAlreadyPresent:

  CreateDirectory "$SMPROGRAMS\GCloud Dot"
  CreateShortcut "$SMPROGRAMS\GCloud Dot\GCloud Dot.lnk" "$INSTDIR\gcloud-dot-tray.exe" "" "$INSTDIR\gcloud-dot.ico"
  CreateShortcut "$SMSTARTUP\GCloud Dot.lnk" "$INSTDIR\gcloud-dot-tray.exe" "" "$INSTDIR\gcloud-dot.ico"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Appear in Settings > Apps, as anything installed should.
  !define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\GCloudDot"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayName"     "GCloud Dot"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayVersion"  "${VERSION}"
  WriteRegStr   HKCU "${UNINST_KEY}" "Publisher"       "Nicholas Glazkov"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayIcon"     "$INSTDIR\gcloud-dot.ico"
  WriteRegStr   HKCU "${UNINST_KEY}" "URLInfoAbout"    "https://nicglazkov.github.io/gcloud-dot/"
  WriteRegStr   HKCU "${UNINST_KEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  nsExec::ExecToLog 'taskkill /IM gcloud-dot-tray.exe /F'
  Pop $0

  Delete "$INSTDIR\gcloud-dot-tray.exe"
  Delete "$INSTDIR\gcloud-dot.exe"
  Delete "$INSTDIR\gcloud-dot.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\GCloud Dot\GCloud Dot.lnk"
  RMDir "$SMPROGRAMS\GCloud Dot"
  Delete "$SMSTARTUP\GCloud Dot.lnk"

  DeleteRegKey HKCU "Software\GCloudDot"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\nicglazkov.GCloudDot"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GCloudDot"

  ; Measured session lengths take days of wall-clock time to gather, so they are
  ; left behind rather than deleted. Reinstalling picks up where this left off;
  ; %LOCALAPPDATA%\GCloudDot can be removed by hand to start clean.
SectionEnd
