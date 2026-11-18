; The app writes its own autostart value ("ContinuityHandoff") under HKCU Run, only when the user turns
; "Start with Windows" on. The stock uninstaller removes a value named after the product
; ("${PRODUCTNAME}", with a space), which is a different name - so remove ours too.
; HKCU only: a per-user install never touches HKLM and needs no admin.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ContinuityHandoff"
!macroend
