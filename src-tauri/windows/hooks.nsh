; NSIS hooks for the Sanco installer (per-machine install, so it runs elevated).
;
; Sync listens on TCP 7470 on all interfaces, but the app itself only answers
; Tailscale addresses (100.64.0.0/10, fd7a:115c:a1e0::/48) and loopback. The
; firewall rule matches: inbound TCP 7470 from Tailscale ranges only.
;
; If someone clicked "Cancel" on a Windows Defender prompt in a dev build, a
; block rule for that exe may exist; block rules win over allow rules. Remove
; it in "Windows Defender Firewall with Advanced Security" → Inbound Rules.

!macro NSIS_HOOK_POSTINSTALL
  ; Replace any rule from a previous install instead of stacking copies.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Sanco sync"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Sanco sync" dir=in action=allow protocol=TCP localport=7470 remoteip=100.64.0.0/10,fd7a:115c:a1e0::/48 program="$INSTDIR\${MAINBINARYNAME}.exe" profile=any enable=yes'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Sanco sync"'
!macroend
