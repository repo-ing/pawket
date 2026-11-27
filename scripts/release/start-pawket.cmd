@echo off
setlocal
if exist "%~dp0media-tools\yt-dlp.exe" set "PATH=%~dp0media-tools;%PATH%"
if not defined LOCALAPPDATA set "LOCALAPPDATA=%USERPROFILE%\AppData\Local"
"%~dp0pawket.exe" serve --open --output "%LOCALAPPDATA%\Pawket"
if errorlevel 1 pause
