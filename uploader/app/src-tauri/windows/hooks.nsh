; The previous current-user NSIS build registered itself as "Guild Uploader"
; with the default manufacturer "raidgroup". This installer's product name is
; a different uninstall key, so the bundled reinstall page does not see it.
; Remove that program copy and its shortcuts. A silent uninstall still
; removes those shortcuts. The delete-data checkbox is left unset.
; Prefs stay in guild-uploader and the sign-in stays in the OS keyring.

!macro NSIS_HOOK_PREINSTALL
  Push $R7
  Push $R8
  Push $R9
  ReadRegStr $R9 HKCU "Software\raidgroup\Guild Uploader" ""
  ReadRegStr $R7 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Guild Uploader" "UninstallString"
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
      MessageBox MB_ICONEXCLAMATION "The earlier Guild Uploader install is still there. Close it, then run this installer again. Saved sign-in and settings were not removed."
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

!macro NSIS_HOOK_POSTINSTALL
  ; The app's Run value is guild-uploader-app, not the display name. The old
  ; uninstaller does not delete it. Point an existing value at this exe.
  ; A missing value means start-at-login is off, so do not create one.
  Push $R8
  ReadRegStr $R8 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "guild-uploader-app"
  ${If} $R8 != ""
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "guild-uploader-app" '"$INSTDIR\guild-uploader-app.exe" --hidden'
  ${EndIf}
  Pop $R8
!macroend
