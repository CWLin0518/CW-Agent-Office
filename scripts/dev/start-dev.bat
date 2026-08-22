@echo off
cd /d "%~dp0..\.."
echo Starting GT Office (dev)...
call npm run dev:tauri
pause
