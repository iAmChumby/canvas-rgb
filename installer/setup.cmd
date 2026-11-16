@echo off
rem Runs setup.ps1 for this session only, without changing the machine's script policy.
rem A .cmd keeps a PowerShell command line off the web page and the clipboard (#79).
rem token: this person's own, unelevated token; setup.ps1 raises its one UAC prompt itself.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1" %*
exit /b %ERRORLEVEL%
