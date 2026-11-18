; The app writes its own autostart value ("Aura") under HKCU Run, only when the user turns
; "Start with Windows" on. The stock uninstaller removes "${PRODUCTNAME}" ("Aura") too, but keep this
; explicit. HKCU only: a per-user install never touches HKLM and needs no admin.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Aura"
!macroend
