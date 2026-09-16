# Codex Hub

Codex Hub 是一个面向 Windows 的本地 Codex 多账号管理工具，用于集中查看账号额度、快速切换账号、管理第三方服务，以及统计本机 Codex 的 Token 使用情况。

> 本项目是非官方工具，与 OpenAI 无隶属或背书关系。账号切换会修改当前 Windows 用户的 Codex 登录文件，请在操作前结束重要任务。

## 功能

- 集中管理多个 Codex 官方账号和 OpenAI 兼容的第三方账号。
- 查看官方账号的 5 小时、每周及代码审查额度。
- 刷新单个或全部账号额度，并在额度耗尽或恢复时发送通知。
- 切换账号前自动备份登录文件，关闭 Codex 相关进程后完成切换并重新启动。
- 支持第三方服务的 Base URL、API Key 和模型列表，可从 `/models` 接口读取模型。
- 通过拖放调整主页账号卡片顺序，支持重命名、隐藏、编辑和删除账号。
- 手动或自动激活尚未开始计时的 5 小时额度窗口。
- 按今天、最近 7 天或最近 30 天统计本机 Codex Token 用量。
- 支持系统托盘、任务栏额度悬浮框、全局快捷键、开机自启和静默启动。
- 设置修改后立即生效，无需额外保存。

## 系统要求

- Windows 10 或 Windows 11。
- 已安装 Codex 桌面应用。
- 使用“激活 5 小时窗口”功能时需要可用的 `codex` CLI。程序会自动查找 Codex 安装目录，也可通过 `CODEX_CLI_PATH` 环境变量指定路径。

## 快速开始

1. 从 `release` 目录运行 `Codex Hub.exe`。
2. 点击“添加账号”。
3. 选择账号类型：
   - 官方账号：读取当前 Codex 登录态、选择 `auth.json` 文件，或粘贴完整授权内容。
   - 第三方账号：填写 API Key、Base URL 和至少一个模型。
4. 保存后刷新额度。
5. 点击账号卡片上的“切换到该账号”，确认后由程序关闭 Codex、切换登录态并重新打开。

官方登录文件通常位于：

```text
%USERPROFILE%\.codex\auth.json
```

## 设置说明

| 设置 | 说明 |
| --- | --- |
| 后台刷新间隔 | 控制额度自动刷新频率，设置为 0 可关闭。 |
| 全局快捷键 | 呼出或收起主窗口；留空可禁用。 |
| 开启自启 | 登录 Windows 后自动启动 Codex Hub。 |
| 静默启动 | 仅在开机自启时不显示主窗口，手动启动仍正常显示。 |
| 后台运行 | 关闭主窗口时驻留后台，不直接退出。 |
| 任务栏悬浮框 | 在任务栏上方常驻显示当前账号额度，单击打开主窗口，右键打开账号菜单。 |
| 自动激活未启动的 5h 窗口 | 刷新时发现官方账号的 5 小时窗口未开始计时，会通过 CLI 发起一次极简只读会话。 |
| 额度通知 | 可分别控制额度耗尽和额度恢复提醒。 |

“自动激活”使用独立的临时 `CODEX_HOME`，不会切换当前登录账号。临时目录在任务结束后删除；同一账号 30 分钟内不会重复激活。若每周额度已经耗尽，会话无法触发 5 小时窗口，程序会先跳过该账号，并在周额度恢复后的刷新中自动重试。

## 数据与安全

Codex Hub 的数据目录为：

```text
%USERPROFILE%\.codex-hub\
├── vault.enc       # 加密账号库
└── backups\        # 切换账号前生成的 auth.json 备份
```

- 账号库使用 AES-256-GCM 整体加密。
- 主密钥保存在 Windows 凭据管理器中，仅供当前 Windows 用户读取。
- 切换官方账号会改写 `%USERPROFILE%\.codex\auth.json`。
- 切换第三方账号时还会更新 Codex Hub 托管的模型服务配置，但会保留项目、会话和其他用户设置。
- Token 统计只读取本机 `.codex/sessions` 与 `.codex/archived_sessions` 中的会话记录，并按调用记录去重。

不要把 `auth.json`、API Key、`vault.enc` 或账号库备份提交到版本控制或发送给他人。

## 本地开发

需要预先安装：

- Node.js 与 npm
- Rust stable 工具链
- Visual Studio C++ Build Tools
- Tauri 2 在 Windows 上所需的 WebView2 和系统依赖

安装前端依赖：

```powershell
npm install
```

启动开发模式：

```powershell
.\run.cmd
```

也可以直接运行：

```powershell
npm run tauri dev
```

执行验证：

```powershell
npm run build
cd src-tauri
cargo test
```

构建便携版程序：

```powershell
.\publish.cmd
```

构建结果会复制到：

```text
release\Codex Hub.exe
```

## 项目结构

```text
src/                 React + TypeScript 前端
src-tauri/src/       Rust 后端、账号切换、额度查询与系统集成
src-tauri/            Tauri 配置与图标资源
tools/                本地构建辅助脚本
release/              发布版可执行文件
```

## 常见问题

### 刷新额度提示登录凭证失效

如果出现 `invalid_grant` 或 refresh token 已被使用，通常表示其他 Codex 实例已经轮换了登录凭证。请在 Codex 中重新登录该账号，再将最新登录态更新到账号库。

### 切换账号时为什么要重启 Codex

Codex 会在启动时读取登录态，并可能在运行期间更新 Token。先关闭相关进程、再写入登录文件，可以避免旧进程覆盖刚切换的新登录态。

### 静默启动后如何打开主窗口

可以单击系统托盘图标、任务栏悬浮框，或使用设置中的全局快捷键。
