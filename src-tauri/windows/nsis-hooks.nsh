; Override the .lados file association icon (Tauri defaults to the app exe icon).

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\Lados Vault\DefaultIcon" "" "$\"$INSTDIR\lados-file.ico$\",0"
  System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend
