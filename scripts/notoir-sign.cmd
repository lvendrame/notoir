@echo off
pwsh -NoProfile -NonInteractive -File "%~dp0sign-windows.ps1" -Path "%~1"
exit /b %ERRORLEVEL%
