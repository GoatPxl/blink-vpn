; NSIS installer hooks, wired in through bundle.windows.nsis.installerHooks in tauri.conf.json.

; Launch at sign-in lives in the registry, not in the install folder, so removing the app would
; otherwise leave Windows trying to start a program that no longer exists. Value names match
; src-tauri/src/backend/autostart.rs.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Blink"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Blink"
!macroend
