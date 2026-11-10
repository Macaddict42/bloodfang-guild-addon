; Earlier installs used different display names, so they are different uninstall
; keys and the bundled reinstall page does not see them. Remove those program
; copies and their shortcuts. A silent uninstall leaves the delete-data
; checkbox unset. Prefs stay in guild-uploader and the sign-in stays in the
; OS keyring. The bundle identifier is unchanged.

!macro RemoveOnePrevious REGPATH UNINSTKEY LEFTOVER
  Push $R7
  Push $R8
  Push $R9
  ReadRegStr $R9 HKCU "${REGPATH}" ""
  ReadRegStr $R7 HKCU "${UNINSTKEY}" "UninstallString"
  ${If} $R9 != ""
  ${AndIf} $R7 != ""
  ${AndIf} ${FileExists} "$R9\uninstall.exe"
    ClearErrors
    ExecWait '"$R9\uninstall.exe" /S _?=$R9' $R8
    ${If} ${Errors}
    ${OrIf} $R8 <> 0
      Pop $R9
      Pop $R8
      Pop $R7
      MessageBox MB_ICONEXCLAMATION "${LEFTOVER}"
      Abort
    ${EndIf}
    ; _?= leaves the uninstaller and the directory behind. Remove only those.
    Delete "$R9\uninstall.exe"
    RMDir "$R9"
  ${EndIf}
  Pop $R9
  Pop $R8
  Pop $R7
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro RemoveOnePrevious "Software\raidgroup\Guild Uploader" "Software\Microsoft\Windows\CurrentVersion\Uninstall\Guild Uploader" "The earlier Guild Uploader install is still there. Close it, then run this installer again. Saved sign-in and settings were not removed."
  !insertmacro RemoveOnePrevious "Software\Bloodfang Guild\Bloodfang App" "Software\Microsoft\Windows\CurrentVersion\Uninstall\Bloodfang App" "The earlier Bloodfang App install is still there. Close it, then run this installer again. Saved sign-in and settings were not removed."
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; The stable Run value name is guild-uploader-app, not the display name.
  ; Point an existing value at this executable. A missing value means start
  ; at login is off, so do not create one. Drop values stored under an old
  ; display name so those paths are not launched as well.
  Push $R8
  ReadRegStr $R8 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "guild-uploader-app"
  ${If} $R8 != ""
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "guild-uploader-app" '"$INSTDIR\Bloodfang Uploader.exe" --hidden'
  ${EndIf}
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Bloodfang App"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Guild Uploader"
  ; 0.1.0 shipped the update checker as guild-uploader-verify-update.exe. It is
  ; updater.exe now, so drop the old copy left in the install directory.
  Delete "$INSTDIR\guild-uploader-verify-update.exe"
  Pop $R8
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; The template removes a Run value named after the display name. This app's
  ; stable value is guild-uploader-app. Leave it in place during an update.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "guild-uploader-app"
  ${EndIf}
!macroend
