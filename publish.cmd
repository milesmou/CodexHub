@echo off
chcp 65001 >nul
rem ============================================================
rem  Codex Hub —— 构建 release EXE
rem
rem  产物：
rem    release\Codex Hub.exe
rem
rem  release 开了 LTO，构建大约 10-15 分钟。
rem ============================================================
setlocal
cd /d "%~dp0"

where npm >nul 2>nul
if errorlevel 1 goto nonpm

powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\tools\prepare-build.ps1"
if errorlevel 1 goto failed

:havenpm
if exist "node_modules" goto havenv

echo [publish] 正在安装前端依赖...
call npm install
if errorlevel 1 goto failed

:havenv
echo.
echo [publish] 开始构建 release 版本（LTO 全量优化，约 10-15 分钟）...
echo.
call npx tauri build --no-bundle
if errorlevel 1 goto failed

rem 构建成功后更新发布文件
if not exist "release" mkdir "release"
if errorlevel 1 goto copyfailed

copy /y "src-tauri\target\release\codex-hub.exe" "release\Codex Hub.exe" >nul
if errorlevel 1 goto copyfailed

echo.
echo [publish] 构建完成。
echo.
echo   发布目录： release\
echo     Codex Hub.exe
echo.
pause
exit /b 0

:nonpm
echo.
echo [publish] 找不到 npm。请先安装 Node.js 并将 npm 加入 PATH。
pause
exit /b 1

:failed
echo.
echo [publish] 构建失败，请看上面的错误信息。
pause
exit /b 1

:copyfailed
echo.
echo [publish] 构建成功，但复制 EXE 到 release 目录失败。
pause
exit /b 1
