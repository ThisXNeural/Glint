; Installer for Glint. One setup with two ways to install, picked on the first page:
;   Just for me  - no administrator rights, indexes your user folder.
;   Everyone     - asks for administrator rights once, indexes every drive live
;                  through the NTFS journal and starts at sign-in through a task.
; Silent installs: GlintSetup.exe /S [/MODE=admin]
; Expects Glint.exe and glint.ico next to this script; VERSION comes from build.sh.

Target amd64-unicode
Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
!include "MUI2.nsh"
!include "nsDialogs.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define APP "Glint"
!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!define PUBLISHER "Glint"
!define TASK "Glint"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Glint"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${APP}"
OutFile "GlintSetup.exe"
BrandingText "${APP} ${VERSION}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP} setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "MIT License"

Var Mode        ; "user" or "admin"
Var RadioUser
Var RadioAdmin

!define MUI_ICON "glint.ico"
!define MUI_UNICON "glint.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Glint.exe"
Page custom ModePage ModePageLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Farsi"

LangString ModeTitle ${LANG_ENGLISH} "Choose how to install"
LangString ModeTitle ${LANG_FARSI} "نوع نصب را انتخاب کنید"
LangString ModeSub ${LANG_ENGLISH} "You can change this later by running setup again."
LangString ModeSub ${LANG_FARSI} "بعداً با اجرای دوبارهٔ نصب می‌توانید آن را تغییر دهید."
LangString UserLabel ${LANG_ENGLISH} "Just for me (recommended, no administrator rights)"
LangString UserLabel ${LANG_FARSI} "فقط برای من (پیشنهادی، بدون نیاز به دسترسی ادمین)"
LangString UserHelp ${LANG_ENGLISH} "Searches apps and the files in your user folder."
LangString UserHelp ${LANG_FARSI} "برنامه‌ها و فایل‌های پوشهٔ کاربری شما را جستجو می‌کند."
LangString AdminLabel ${LANG_ENGLISH} "Full disk search (needs administrator rights)"
LangString AdminLabel ${LANG_FARSI} "جستجوی کل دیسک (نیاز به دسترسی ادمین)"
LangString AdminHelp ${LANG_ENGLISH} "Searches every drive instantly through the NTFS index and starts at sign-in without prompts."
LangString AdminHelp ${LANG_FARSI} "همهٔ درایوها را از طریق ایندکس NTFS فوراً جستجو می‌کند و هنگام ورود بدون پرسش اجرا می‌شود."

Function .onInit
  StrCpy $Mode "user"
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/MODE=" $1
  ${IfNot} ${Errors}
    StrCpy $Mode $1
  ${EndIf}
  ; Started again with rights for the full disk install.
  ${If} $Mode == "admin"
    UserInfo::GetAccountType
    Pop $2
    ${If} $2 != "Admin"
      MessageBox MB_ICONSTOP "Administrator rights are needed for the full disk install."
      Abort
    ${EndIf}
    SetShellVarContext all
    StrCpy $INSTDIR "$PROGRAMFILES64\${APP}"
  ${EndIf}
FunctionEnd

Function ModePage
  ; The elevated copy already knows its mode.
  ${If} $Mode == "admin"
    Abort
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "$(ModeTitle)" "$(ModeSub)"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateRadioButton} 0 6u 100% 12u "$(UserLabel)"
  Pop $RadioUser
  ${NSD_CreateLabel} 12u 20u 95% 20u "$(UserHelp)"
  Pop $0
  ${NSD_CreateRadioButton} 0 50u 100% 12u "$(AdminLabel)"
  Pop $RadioAdmin
  ${NSD_CreateLabel} 12u 64u 95% 28u "$(AdminHelp)"
  Pop $0
  ${NSD_Check} $RadioUser
  nsDialogs::Show
FunctionEnd

Function ModePageLeave
  ${NSD_GetState} $RadioAdmin $0
  ${If} $0 == ${BST_CHECKED}
    ; Run this setup again with administrator rights, then step aside.
    HideWindow
    ExecShellWait "runas" "$EXEPATH" "/MODE=admin" SW_SHOWNORMAL
    Quit
  ${EndIf}
FunctionEnd

Function un.onInit
  ReadRegStr $Mode HKLM "${UNINST_KEY}" "InstallMode"
  ${If} $Mode == "admin"
    SetShellVarContext all
  ${EndIf}
FunctionEnd

Section "Install"
  ; Replace a running copy, and the copy from the other mode if present.
  nsExec::Exec 'taskkill /F /IM Glint.exe'
  Sleep 300
  ${If} $Mode == "admin"
    ReadRegStr $0 HKCU "${UNINST_KEY}" "UninstallString"
    ${If} $0 != ""
      ExecWait '$0 /S _?=$LOCALAPPDATA\Programs\${APP}'
      Delete "$LOCALAPPDATA\Programs\${APP}\Uninstall.exe"
      RMDir "$LOCALAPPDATA\Programs\${APP}"
    ${EndIf}
  ${EndIf}

  SetOutPath "$INSTDIR"
  File "Glint.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\Glint.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\Glint.exe"

  ${If} $Mode == "admin"
    StrCpy $R0 HKLM
    ; A logon task with full rights reads the NTFS index without a UAC prompt
    ; at every sign-in. No time limit, and keeps running on battery.
    InitPluginsDir
    ExpandEnvStrings $0 "%USERDOMAIN%\%USERNAME%"
    FileOpen $1 "$PLUGINSDIR\task.xml" w
    FileWriteWord $1 0xFEFF
    FileWriteUTF16LE $1 '<?xml version="1.0" encoding="UTF-16"?>$\r$\n'
    FileWriteUTF16LE $1 '<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">'
    FileWriteUTF16LE $1 '<RegistrationInfo><Description>Starts Glint at sign-in.</Description></RegistrationInfo>'
    FileWriteUTF16LE $1 '<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>$0</UserId></LogonTrigger></Triggers>'
    FileWriteUTF16LE $1 '<Principals><Principal id="Author"><UserId>$0</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>'
    FileWriteUTF16LE $1 '<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>5</Priority><StartWhenAvailable>true</StartWhenAvailable></Settings>'
    FileWriteUTF16LE $1 '<Actions Context="Author"><Exec><Command>"$INSTDIR\Glint.exe"</Command><Arguments>--background</Arguments></Exec></Actions>'
    FileWriteUTF16LE $1 '</Task>'
    FileClose $1
    nsExec::Exec 'schtasks /Create /F /TN "${TASK}" /XML "$PLUGINSDIR\task.xml"'
    ; Tells Glint.exe to start through the task when opened from the Start menu.
    FileOpen $1 "$INSTDIR\elevated-task" w
    FileClose $1
    WriteRegStr HKLM "${UNINST_KEY}" "InstallMode" "admin"
    WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\Glint.exe"
    WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
    WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" 10240
  ${Else}
    WriteRegStr HKCU "${RUN_KEY}" "${TASK}" '"$INSTDIR\Glint.exe" --background'
    WriteRegStr HKCU "${UNINST_KEY}" "InstallMode" "user"
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\Glint.exe"
    WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
    WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" 10240
  ${EndIf}
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM Glint.exe'
  Sleep 300
  ${If} $Mode == "admin"
    nsExec::Exec 'schtasks /Delete /F /TN "${TASK}"'
    DeleteRegKey HKLM "${UNINST_KEY}"
  ${Else}
    DeleteRegValue HKCU "${RUN_KEY}" "${TASK}"
    DeleteRegKey HKCU "${UNINST_KEY}"
  ${EndIf}
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  Delete "$INSTDIR\Glint.exe"
  Delete "$INSTDIR\elevated-task"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
