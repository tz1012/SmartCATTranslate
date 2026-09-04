; Preserve the existing install directory while changing the public product name.
; The legacy registry key is removed only after the new installer has written a
; valid BYOK Translator entry and the application executable exists.

!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R8 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartCAT Translate" "InstallLocation"
  StrCmp $R8 "" byok_rebrand_preinstall_done
  StrCpy $R9 $R8 1
  StrCmp $R9 "$\"" 0 +2
  StrCpy $R8 $R8 -1 1
  IfFileExists "$R8\smartcat-translate.exe" 0 byok_rebrand_preinstall_done
  StrCpy $INSTDIR "$R8"
  SetOutPath "$INSTDIR"
byok_rebrand_preinstall_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $R8 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartCAT Translate" "InstallLocation"
  StrCmp $R8 "" byok_rebrand_postinstall_done
  StrCpy $R9 $R8 1
  StrCmp $R9 "$\"" 0 +2
  StrCpy $R8 $R8 -1 1
  StrCmp $R8 $INSTDIR 0 byok_rebrand_postinstall_done
  IfFileExists "$INSTDIR\smartcat-translate.exe" 0 byok_rebrand_postinstall_done
  ReadRegStr $R9 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\BYOK Translator" "DisplayVersion"
  StrCmp $R9 "" byok_rebrand_postinstall_done
  DeleteRegKey SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartCAT Translate"
byok_rebrand_postinstall_done:
!macroend
