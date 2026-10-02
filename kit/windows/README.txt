Pulse - Windows test kit
========================

Runs everything on this PC, reachable only from this PC (127.0.0.1).

1. Unzip this folder somewhere (e.g. Documents\pulse-kit).
2. Double-click start.cmd.
   - First run prints two invite codes. In the app, click "Got an invite? Create an account",
     set Server to http://127.0.0.1:7890 and paste a code.
   - For a second user at the same time: open a terminal here and run
       start.cmd -Second
3. Press Enter in the start window to stop the server and LiveKit.

Windows may warn about unsigned apps (SmartScreen: "More info" -> "Run anyway").
If Smart App Control blocks them outright, it has to be turned off to test.

Logs
  App:          %LOCALAPPDATA%\app.pulse.client\logs  (second profile: ...\logs\profiles\b)
  Server/LK:    data\*.log next to this README
  To share:     right-click collect-logs.ps1 -> Run with PowerShell; a zip lands on the Desktop.

Also included: the installer (Pulse_*_x64-setup.exe) to test a real install.
