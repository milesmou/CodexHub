@echo off
chcp 65001 >nul
rem ============================================================
rem  Codex 多账号管家 —— 构建 release 版本 + 安装包
rem
rem  产物：
rem    src-tauri\target\release\codex-helper.exe
rem    src-tauri\target\release\bundle\nsis\CodexHelper_x.x.x_x64-setup.exe
rem
rem  release 开了 LTO，构建大约 10-15 分钟。
rem  只想快速拿个 exe、不要安装包：publish.cmd --no-bundle
rem ============================================================
setlocal
cd /d "%~dp0"

set "BUNDLE_ARGS="
if /i "%~1"=="--no-bundle" set "BUNDLE_ARGS=--no-bundle"

where npm >nul 2>nul
if not errorlevel 1 goto havenpm

rem 系统 PATH 里没有 npm，退回本机托管的 Node
set "NODE_DIR=C:\Users\Administrator\.workbuddy\binaries\node\versions\22.22.2-2"
if not exist "%NODE_DIR%\npm.cmd" goto nonpm
set "PATH=%NODE_DIR%;%PATH%"
echo [publish] 系统未找到 npm，已临时使用托管 Node：%NODE_DIR%

:havenpm
if exist "node_modules" goto havenv

echo [publish] 正在安装前端依赖...
call npm install
if errorlevel 1 goto failed

:havenv
echo.
echo [publish] 开始构建 release 版本（LTO 全量优化，约 10-15 分钟）...
echo.
call npx tauri build %BUNDLE_ARGS%
if errorlevel 1 goto failed

echo.
echo [publish] 构建完成。
echo.
echo   可执行文件： src-tauri\target\release\codex-helper.exe
if "%BUNDLE_ARGS%"=="" (
  echo   安装包目录： src-tauri\target\release\bundle\nsis\
  if exist "src-tauri\target\release\bundle\nsis" (
    for %%F in ("src-tauri\target\release\bundle\nsis\*.exe") do echo     %%~nxF
  )
)
echo.
pause
exit /b 0

:nonpm
echo.
echo [publish] 找不到 npm。请先安装 Node.js，或修改本脚本里的 NODE_DIR。
pause
exit /b 1

:failed
echo.
echo [publish] 构建失败，请看上面的错误信息。
pause
exit /b 1
