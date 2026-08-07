@echo off
setlocal

set "LOG_DIR=test-results"
set "LOG_FILE=%LOG_DIR%\tauri-real-dev.log"

if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"

echo ==================================================>> "%LOG_FILE%"
echo [%date% %time%] Starting tauri:dev:e2e>> "%LOG_FILE%"

powershell -NoProfile -ExecutionPolicy Bypass -Command "npm run tauri:dev:e2e 2>&1 | Tee-Object -FilePath '%LOG_FILE%' -Append; exit $LASTEXITCODE"
set "EXIT_CODE=%ERRORLEVEL%"

echo [%date% %time%] tauri:dev:e2e exited with %EXIT_CODE%>> "%LOG_FILE%"
exit /b %EXIT_CODE%