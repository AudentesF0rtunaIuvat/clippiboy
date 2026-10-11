; Installer hooks, wired up in tauri.conf.json (bundle.windows.nsis.installerHooks).
; Kept to plain ASCII: NSIS reads a script without BOM in the system code page.

; Runs before anything is copied.
;
; After an update from inside the app, the old ClippiBoy is still on its way
; out: the updater starts this installer first and only then ends the process,
; and taking down capture, audio and the GPU encoder takes a moment. The app
; check that follows can miss a process in that state, and copying over
; clippiboy.exe then fails with "Error opening file for writing".
;
; So wait here until the file can be opened for writing again - at most 20 s.
; Past that, the usual check takes over and shuts down whatever still holds it.
!macro NSIS_HOOK_PREINSTALL
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    Push $R8
    Push $R9
    StrCpy $R9 0
    clippiboy_wait_for_exe:
      ClearErrors
      FileOpen $R8 "$INSTDIR\${MAINBINARYNAME}.exe" a
      ${IfNot} ${Errors}
        FileClose $R8
        Goto clippiboy_exe_free
      ${EndIf}
      IntOp $R9 $R9 + 1
      IntCmp $R9 80 clippiboy_exe_free
      Sleep 250
      Goto clippiboy_wait_for_exe
    clippiboy_exe_free:
    Pop $R9
    Pop $R8
  ${EndIf}
!macroend

; Runs after everything is in place.
;
; Downloads from clippiboy.com/get/<ref> are named ...-setup.<ref>.exe, so the
; installer's own file name says which link it came from. Write it down for the
; app, which reads the source out of it (telemetry.rs) - no parsing here. Only
; the first install does: an update runs the plain GitHub installer and must not
; overwrite where someone originally came from.
!macro NSIS_HOOK_POSTINSTALL
  ${IfNot} ${FileExists} "$LOCALAPPDATA\gg.clippiboy.app\install-source.txt"
    Push $R8
    CreateDirectory "$LOCALAPPDATA\gg.clippiboy.app"
    ClearErrors
    FileOpen $R8 "$LOCALAPPDATA\gg.clippiboy.app\install-source.txt" w
    ${IfNot} ${Errors}
      FileWrite $R8 "$EXEFILE"
      FileClose $R8
    ${EndIf}
    Pop $R8
  ${EndIf}
!macroend
