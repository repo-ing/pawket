@echo off
setlocal
if not defined LOCALAPPDATA set "LOCALAPPDATA=%USERPROFILE%\AppData\Local"
"%~dp0pawket.exe" serve --open --output "%LOCALAPPDATA%\Pawket"
if errorlevel 1 pause
