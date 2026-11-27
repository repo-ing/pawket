Unicode true
Name "Pawket"
OutFile "${OUTPUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Pawket"
SetCompressor /SOLID lzma
!include "MUI2.nsh"
!define MUI_FINISHPAGE_RUN "$INSTDIR\Start Pawket.cmd"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchPawket
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_LANGUAGE "English"

Function LaunchPawket
  ExecShell "open" "$INSTDIR\Start Pawket.cmd"
FunctionEnd

Section
  SetOutPath "$INSTDIR"
  File "${STAGE}/pawket.exe"
  File "${STAGE}/Start Pawket.cmd"
  File "${STAGE}/LICENSE"
  SetOutPath "$INSTDIR\media-tools"
  File "${STAGE}/tools/*.exe"
  SetOutPath "$INSTDIR\licenses"
  File "${STAGE}/licenses/*"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Pawket.lnk" "$INSTDIR\Start Pawket.cmd" "" "$INSTDIR\pawket.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pawket" "DisplayName" "Pawket"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pawket" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\Pawket.lnk"
  Delete "$INSTDIR\media-tools\ffmpeg.exe"
  Delete "$INSTDIR\media-tools\yt-dlp.exe"
  Delete "$INSTDIR\media-tools\deno.exe"
  RMDir "$INSTDIR\media-tools"
  Delete "$INSTDIR\licenses\FFmpeg-GPLv2.txt"
  Delete "$INSTDIR\licenses\FFmpeg-GPLv3.txt"
  Delete "$INSTDIR\licenses\deno.txt"
  Delete "$INSTDIR\licenses\yt-dlp.txt"
  RMDir "$INSTDIR\licenses"
  Delete "$INSTDIR\pawket.exe"
  Delete "$INSTDIR\Start Pawket.cmd"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pawket"
SectionEnd
