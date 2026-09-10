@echo off
chcp 65001 >nul
rem ============================================================
rem  CodexHelper —— debug 模式运行（开发 / 验收用）
rem
rem  会做四件事：
rem    1. 确认能找到 npm
rem    2. 首次运行时自动安装前端依赖
rem    3. 清掉上次残留的进程（否则 1420 端口被占会起不来）
rem    4. 用 tauri dev 启动：Vite 热更新 + Rust 增量编译
rem
rem  注意：首次 Rust 编译要几分钟，之后就快了。
rem  要出正式版请用 publish.cmd。
rem ============================================================
setlocal
cd /d "%~dp0"

where npm >nul 2>nul
if not errorlevel 1 goto havenpm

rem 系统 PATH 里没有 npm，退回本机托管的 Node
set "NODE_DIR=C:\Users\Administrator\.workbuddy\binaries\node\versions\22.22.2-2"
if not exist "%NODE_DIR%\npm.cmd" goto nonpm
set "PATH=%NODE_DIR%;%PATH%"
echo [run] 系统未找到 npm，已临时使用托管 Node：%NODE_DIR%

:havenpm
if exist "node_modules" goto havenv

echo [run] 首次运行，正在安装前端依赖...
call npm install
if errorlevel 1 goto failed

:havenv
call :killstale

echo.
echo [run] 以 debug 模式启动 CodexHelper
echo       首次 Rust 编译需要几分钟，请耐心等待
echo       关掉本窗口或按 Ctrl+C 结束
echo.
call npx tauri dev
echo.
echo [run] 已退出。
goto end

rem ------------------------------------------------------------
rem  清理上一次没退干净的进程：
rem    - codex-helper.exe：上一次的应用实例
rem    - 占用 1420 端口的进程：上一次 Vite dev server
rem  只针对这两个，不动别的 node 进程。
rem ------------------------------------------------------------
:killstale
taskkill /F /IM codex-helper.exe >nul 2>nul
for /f "tokens=5" %%p in ('netstat -ano ^| findstr ":1420 " ^| findstr "LISTENING"') do (
    taskkill /F /PID %%p >nul 2>nul
)
timeout /t 1 /nobreak >nul
exit /b 0

:nonpm
echo.
echo [run] 找不到 npm。请先安装 Node.js，或修改本脚本里的 NODE_DIR。
pause
exit /b 1

:failed
echo.
echo [run] 前端依赖安装失败，请看上面的错误信息。
pause
exit /b 1

:end
endlocal
