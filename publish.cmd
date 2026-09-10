@echo off
chcp 65001 >nul
rem ============================================================
rem  Codex 多账号管家 —— 构建 release EXE
rem
rem  产物：
rem    release\codex-helper.exe
rem
rem  release 开了 LTO，构建大约 10-15 分钟。
rem ============================================================
setlocal
cd /d "%~dp0"

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
call npx tauri build --no-bundle
if errorlevel 1 goto failed

rem 构建成功后再刷新发布目录，避免构建失败时删除上一次的可用产物
if exist "release" rmdir /s /q "release"
if errorlevel 1 goto copyfailed
mkdir "release"
if errorlevel 1 goto copyfailed

copy /y "src-tauri\target\release\codex-helper.exe" "release\codex-helper.exe" >nul
if errorlevel 1 goto copyfailed

echo.
echo [publish] 构建完成。
echo.
echo   发布目录： release\
echo     codex-helper.exe
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

:copyfailed
echo.
echo [publish] 构建成功，但复制 EXE 到 release 目录失败。
pause
exit /b 1
