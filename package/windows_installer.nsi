; Example NSIS script - Prettier Version
Name SamRewritten
!define APP_NAME "SamRewritten"
!define APP_VERSION "1.6.0"
!define APP_PUBLISHER "Sam Authors"
!define APP_EXE "samrewritten.exe"
!define APP_EXE_ADW "samrewritten-adw.exe"
!define APP_EXE_CLI "samrewritten-cli.exe"

; --- Installer Configuration ---
Outfile "SamRewritten-installer.exe"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
RequestExecutionLevel admin ; Request application privileges

; --- User Interface Enhancements ---
; Modern UI Welcome and Finish pages
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "WinMessages.nsh"
!include "Sections.nsh"

Var RunExe

!define MUI_ICON "..\assets\installer.ico"
!define MUI_UNICON "..\assets\icon.ico"
; !define MUI_WELCOMEFINISH_BMPS ".\installer_welcome.bmp" ; Optional: path to a custom welcome bitmap (164x314 pixels)
; !define MUI_UNWELCOMEFINISH_BMPS ".\installer_uninstall.bmp" ; Optional: path to a custom uninstall bitmap
; !define MUI_ABORTWARNING ; Show a warning if the user tries to cancel
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunSelected
!define MUI_FINISHPAGE_RUN_TEXT "Run SamRewritten now"

; Installer pages
!insertmacro MUI_PAGE_WELCOME
!define MUI_COMPONENTSPAGE_TEXT_TOP "Choose which version of SamRewritten to install. The graphical versions do the same thing and only differ in appearance; select both if you want to compare them."
!define MUI_COMPONENTSPAGE_TEXT_COMPLIST "Select the components to install:"
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE ComponentsLeave
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_SHOW FinishShow
!insertmacro MUI_PAGE_FINISH

; Uninstaller pages
!insertmacro MUI_UNPAGE_WELCOME
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

; Language selection (optional, but good for a "prettier" installer)
!insertmacro MUI_LANGUAGE "English"

; --- Uninstall any previous version, so a new install never mixes in stale files ---
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

Function .onInit
  ReadRegStr $R0 HKLM "${UNINST_KEY}" "UninstallString"
  ReadRegStr $R1 HKLM "${UNINST_KEY}" "InstallLocation"
  ${If} $R0 == ""
  ${OrIf} $R1 == ""
    Return
  ${EndIf}

  MessageBox MB_YESNO|MB_ICONQUESTION "${APP_NAME} is already installed in:$\n$R1$\n$\nRemove that version first? Recommended - leftover files from an older release can prevent the new one from starting." /SD IDYES IDNO keep

  ; _?= runs the uninstaller in place so ExecWait actually waits for it
  ExecWait '"$R0" /S _?=$R1'
  Delete "$R0"
  RMDir "$R1"

keep:
FunctionEnd

; --- Optional components ---
Section "GTK interface" SecGtk
  SetOutPath $INSTDIR
  File "..\SamRewritten-windows-x86_64\${APP_EXE}"
SectionEnd

Section /o "Adwaita interface" SecAdw
  SetOutPath $INSTDIR
  File "..\SamRewritten-windows-x86_64\${APP_EXE_ADW}"
SectionEnd

Section /o "Command-line interface" SecCli
  SetOutPath $INSTDIR
  File "..\SamRewritten-windows-x86_64\${APP_EXE_CLI}"
SectionEnd

; --- Shared payload ---
Section "-Install"
  SetOutPath $INSTDIR
  File "..\assets\README.txt"
  File "..\LICENSE"

  ; The GTK runtime is dead weight for a CLI-only install
  ${If} ${SectionIsSelected} ${SecGtk}
  ${OrIf} ${SectionIsSelected} ${SecAdw}
    SetOutPath "$INSTDIR\share"
    File /r /x "icon-theme.cache" "..\SamRewritten-windows-x86_64\share\*.*"

    SetOutPath "$INSTDIR\lib"
    File /r "..\SamRewritten-windows-x86_64\lib\*.*"

    SetOutPath $INSTDIR
    File /a "..\SamRewritten-windows-x86_64\bin\*.*" ; /a includes all files and subdirectories

    ExecWait '"$INSTDIR\gtk4-update-icon-cache.exe" -f -t "$INSTDIR\share\icons\hicolor"'
    ExecWait '"$INSTDIR\gtk4-update-icon-cache.exe" -f -t "$INSTDIR\share\icons\Adwaita"'
  ${EndIf}

  ; Create uninstaller
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd

; Named after what the user picked: a lone interface is just "SamRewritten"
Section "-Shortcuts"
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"

  ; Reinstalling with a different selection must not leave the old naming behind
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME} (GTK).lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME} (Adwaita).lnk"

  ${If} ${SectionIsSelected} ${SecGtk}
  ${AndIf} ${SectionIsSelected} ${SecAdw}
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME} (GTK).lnk" "$INSTDIR\${APP_EXE}"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME} (Adwaita).lnk" "$INSTDIR\${APP_EXE_ADW}"
    StrCpy $RunExe "${APP_EXE}"
  ${ElseIf} ${SectionIsSelected} ${SecAdw}
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE_ADW}"
    StrCpy $RunExe "${APP_EXE_ADW}"
  ${ElseIf} ${SectionIsSelected} ${SecGtk}
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
    StrCpy $RunExe "${APP_EXE}"
  ${Else}
    RMDir "$SMPROGRAMS\${APP_NAME}"
  ${EndIf}
SectionEnd

; --- Component descriptions ---
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecGtk} "The default SamRewritten experience. Pick this one if you are not sure."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAdw} "The same application with a more modern look and feel, built with libadwaita."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecCli} "Terminal version for scripting and automation. No window and no shortcut: run samrewritten-cli.exe from PowerShell or cmd."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; --- Require at least one version ---
Function ComponentsLeave
  ${IfNot} ${SectionIsSelected} ${SecGtk}
  ${AndIfNot} ${SectionIsSelected} ${SecAdw}
  ${AndIfNot} ${SectionIsSelected} ${SecCli}
    MessageBox MB_ICONEXCLAMATION|MB_OK "Please select at least one version of ${APP_NAME} to install."
    Abort
  ${EndIf}
FunctionEnd

Function RunSelected
  ${If} $RunExe != ""
    ExecShell "" "$INSTDIR\$RunExe"
  ${EndIf}
FunctionEnd

; A CLI-only install has no window to open, so drop the "run now" checkbox
Function FinishShow
  ${If} $RunExe == ""
    SendMessage $mui.FinishPage.Run ${BM_SETCHECK} ${BST_UNCHECKED} 0
    ShowWindow $mui.FinishPage.Run ${SW_HIDE}
  ${EndIf}
FunctionEnd

; --- "Launch now" Checkbox ---
Function .onInstSuccess
  ; Add a checkbox to launch the application
  ; !insertmacro MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
  ; !insertmacro MUI_FINISHPAGE_RUN_TEXT "Launch ${APP_NAME} now"
FunctionEnd

; --- Uninstaller Section ---
Section "Uninstall"
  Delete "$INSTDIR\*.*"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\lib"
  RMDir /r "$INSTDIR\share"
  RMDir "$INSTDIR"

  ; Remove start menu shortcut
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME} (GTK).lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME} (Adwaita).lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"

  ; Remove the uninstaller's registry key
  DeleteRegKey HKLM "${UNINST_KEY}"
SectionEnd