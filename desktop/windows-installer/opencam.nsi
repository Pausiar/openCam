; Instalador de OpenCam para Windows de 64 bits.
;
; Empaqueta el cliente de escritorio y el filtro DirectShow de la cámara virtual,
; y registra el filtro al instalar (y lo desregistra al desinstalar), para no
; tener que pasar a mano por `regsvr32`.
;
; Compilar desde esta carpeta, con las dos piezas dentro de `payload\`:
;   makensis /DVERSION=0.1.2 opencam.nsi
; El workflow `release.yml` prepara el payload con los artefactos compilados.

Unicode true
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "x64.nsh"

!ifndef VERSION
  !define VERSION "0.0.0-dev"
!endif
; Carpeta con opencam-desktop.exe y opencam_vcam.dll.
!ifndef PAYLOAD
  !define PAYLOAD "payload"
!endif
; Carpeta donde dejar el instalador.
!ifndef OUTDIR
  !define OUTDIR "."
!endif

!define APP_NAME "OpenCam"
!define CLIENT_EXE "opencam-desktop.exe"
!define VCAM_DLL "opencam_vcam.dll"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenCam"

Name "${APP_NAME} ${VERSION}"
OutFile "${OUTDIR}\opencam-setup-windows-x86_64.exe"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
InstallDirRegKey HKLM "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel admin
BrandingText "${APP_NAME} ${VERSION}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ; El filtro y el cliente son de 64 bits; en un Windows de 32 bits no sirven.
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "OpenCam necesita Windows de 64 bits."
    Abort
  ${EndIf}

  ; Si el cliente está en marcha, Windows no deja reemplazar el .exe.
  nsExec::Exec '"$WINDIR\Sysnative\taskkill.exe" /IM ${CLIENT_EXE} /F'
  Pop $0
FunctionEnd

Section "OpenCam" SecMain
  SectionIn RO

  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "${PAYLOAD}\${CLIENT_EXE}"
  File "${PAYLOAD}\${VCAM_DLL}"

  ; El instalador es un proceso de 32 bits, así que `regsvr32.exe` de `system32`
  ; le quedaría redirigido a la vista de 32 bits del registro. `Sysnative` apunta
  ; siempre al system32 real y registra el filtro para las apps de 64 bits.
  DetailPrint "Registrando la cámara virtual…"
  ExecWait '"$WINDIR\Sysnative\regsvr32.exe" /s "$INSTDIR\${VCAM_DLL}"' $0
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "No se pudo registrar la cámara virtual (código $0).$\r$\nPuedes intentarlo a mano, como administrador:$\r$\nregsvr32 $\"$INSTDIR\${VCAM_DLL}$\""
  ${EndIf}

  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${CLIENT_EXE}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\Desinstalar ${APP_NAME}.lnk" "$INSTDIR\uninstall.exe"
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${CLIENT_EXE}"

  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "OpenCam"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${CLIENT_EXE}"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  nsExec::Exec '"$WINDIR\Sysnative\taskkill.exe" /IM ${CLIENT_EXE} /F'
  Pop $0

  ExecWait '"$WINDIR\Sysnative\regsvr32.exe" /u /s "$INSTDIR\${VCAM_DLL}"'

  Delete "$INSTDIR\${CLIENT_EXE}"
  Delete "$INSTDIR\${VCAM_DLL}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\Desinstalar ${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"

  DeleteRegKey HKLM "${UNINST_KEY}"
SectionEnd
