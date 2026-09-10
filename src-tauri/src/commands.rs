//! Tauri 命令层：前端通过 `invoke` 调用的全部入口。
//!
//! 真正的业务逻辑都写成 `*_inner` 函数，命令只是薄薄一层包装。
//! 这样托盘菜单也能直接复用同一套逻辑，不用重复实现。

use crate::model::*;
use crate::{accounts, ccswitch, codex, codexapp, quota, stats, store, tray, warmup};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// 自动激活的冷却时间：同一账号 30 分钟内只自动点一次。
///
/// 必要性：额度接口有缓存/延迟，刚点着的窗口可能过几分钟才在接口上体现出来，
/// 没有这道闸门就会在每次后台刷新时重复发请求。
const WARMUP_COOLDOWN_SECS: i64 = 1800;

/// 全局共享状态：加密账号库。
pub struct AppState {
    pub vault: Mutex<Vault>,
    /// 账号 id -> 上次自动激活的时刻（unix 秒），用于冷却
    pub warmup_cooldown: Mutex<HashMap<String, i64>>,
}

impl AppState {
    pub fn new(vault: Vault) -> Self {
        Self {
            vault: Mutex::new(vault),
            warmup_cooldown: Mutex::new(HashMap::new()),
        }
    }
}

/// 切换账号的结果。
#[derive(Debug, Serialize)]
pub struct SwitchOutcome {
    pub ok: bool,
    pub message: String,
    /// 切换前的 auth.json 备份路径
    pub backup_path: Option<String>,
    /// cc-switch 状态是否同步成功
    pub ccswitch_synced: bool,
    /// 是否顺带改写了 config.toml（该账号带了配置片段）
    pub config_applied: bool,
    /// config.toml 的备份路径
    pub config_backup_path: Option<String>,
    /// 切换前关掉了几个 Codex 进程
    pub codex_killed: usize,
    /// Codex 桌面应用是否已经重新拉起
    pub codex_restarted: bool,
    /// 重新拉起失败时的原因（切换本身已成功，所以不当作错误上报）
    pub codex_restart_error: Option<String>,
}

/// 导入结果。
#[derive(Debug, Serialize)]
pub struct ImportOutcome {
    pub imported: usize,
    pub skipped: usize,
    pub message: String,
}

// ---------------------------------------------------------------- 内部工具

/// 把账号库转换成前端视图（**不带 auth 凭证**，避免凭证进入 WebView）。
fn to_views(vault: &Vault) -> Vec<AccountView> {
    let mut list: Vec<AccountView> = vault
        .accounts
        .iter()
        .map(|a| {
            let cached = vault.quota_cache.get(&a.id);
            AccountView {
                id: a.id.clone(),
                name: a.name.clone(),
                email: a
                    .email
                    .clone()
                    .or_else(|| cached.and_then(|q| q.email.clone())),
                plan_type: a
                    .plan_type
                    .clone()
                    .or_else(|| cached.and_then(|q| q.plan_type.clone())),
                kind: a.kind,
                is_current: vault.is_current(&a.id),
                quota: cached.cloned(),
                sort_index: a.sort_index,
                hidden: a.hidden,
                source: a.source.clone(),
            }
        })
        .collect();
    list.sort_by(|a, b| a.sort_index.cmp(&b.sort_index).then(a.name.cmp(&b.name)));
    list
}

/// 落盘 + 通知前端 + 重建托盘。任何改动账号库的地方最后都要调它。
///
/// 调用方此时通常**正持有账号库的锁**，所以：
/// - `tray::rebuild` 只接收 `&Vault`，不会二次加锁（`std::sync::Mutex` 不可重入）
/// - 且丢到异步任务里执行，避免在主线程上调用托盘 API 造成阻塞
fn persist(app: &AppHandle, vault: &Vault) -> Result<(), String> {
    store::save(vault).map_err(|e| format!("{e}"))?;
    let _ = app.emit("accounts-changed", ());

    let app_handle = app.clone();
    let snapshot = vault.clone();
    tauri::async_runtime::spawn(async move {
        tray::rebuild(&app_handle, &snapshot);
    });

    Ok(())
}

/// 取账号库快照（避免长时间持锁）。
fn snapshot<T>(app: &AppHandle, f: impl FnOnce(&Vault) -> T) -> T {
    let state = app.state::<AppState>();
    let vault = state.vault.lock().unwrap();
    f(&vault)
}

// ---------------------------------------------------------------- 业务逻辑

/// 刷新额度的核心实现。`ids` 为 None 时刷新全部官方账号。
///
/// 关键点：网络请求期间**不能**持有 Mutex，所以先取快照 → 并发拉取 → 统一写回。
pub async fn refresh_quotas_inner(
    app: &AppHandle,
    ids: Option<Vec<String>>,
) -> Result<Vec<AccountView>, String> {
    // 1. 取出待刷新账号的快照
    let targets: Vec<(String, serde_json::Value)> = snapshot(app, |vault| {
        vault
            .accounts
            .iter()
            .filter(|a| a.kind == AccountKind::Official)
            .filter(|a| match &ids {
                Some(list) => list.contains(&a.id),
                None => true,
            })
            .map(|a| (a.id.clone(), a.auth.clone()))
            .collect()
    });

    let _ = app.emit("refresh-started", ());

    // 2. 并发查询，某一家慢或挂掉不影响其他账号
    let mut handles = Vec::with_capacity(targets.len());
    for (id, auth) in targets {
        handles.push(tauri::async_runtime::spawn(async move {
            let outcome = quota::fetch(&auth).await;
            (id, outcome)
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(pair) = handle.await {
            results.push(pair);
        }
    }

    // 3. 写回缓存，并把接口返回的套餐 / 邮箱 / 新 token 沉淀下来
    let updated = {
        let state = app.state::<AppState>();
        let mut vault = state.vault.lock().unwrap();
        for (id, outcome) in &results {
            if outcome.quota.ok {
                vault.quota_cache.insert(id.clone(), outcome.quota.clone());
            } else {
                // 查询失败时保留旧缓存，但把错误信息记下来
                vault.quota_cache.insert(id.clone(), outcome.quota.clone());
            }

            if let Some(acc) = vault.find_mut(id) {
                if let Some(plan) = &outcome.quota.plan_type {
                    acc.plan_type = Some(plan.clone());
                }
                if let Some(email) = &outcome.quota.email {
                    acc.email = Some(email.clone());
                }
                if let Some(new_auth) = &outcome.refreshed_auth {
                    acc.auth = new_auth.clone();
                }
            }
        }
        persist(app, &vault)?;
        to_views(&vault)
    };

    let _ = app.emit("refresh-finished", ());
    crate::scheduler::check_and_notify(app, &results);

    // 顺手把「从未启动」的 5 小时窗口点着（默认关闭，见设置里的开关）
    maybe_auto_warmup(app);

    Ok(updated)
}

/// 若用户在设置里打开「自动激活」，就把尚未点着的 5 小时窗口点着。
///
/// 这里只负责**发起**，不等结果：一次 CLI 调用要十几秒，不能拖住刷新流程。
/// 做完也**不回头再刷一次额度** —— 那会和「刷新触发激活」形成回环。
/// 窗口一旦真正启动，下次定时刷新就会自然反映到界面上。
fn maybe_auto_warmup(app: &AppHandle) {
    let (enabled, targets) = snapshot(app, |vault| {
        (vault.settings.warmup_auto, warmup::dormant_ids(vault))
    });

    if !enabled || targets.is_empty() {
        return;
    }

    // 冷却过滤。时间戳要在 spawn **之前**就打上：
    // 否则任务还在跑的时候又来了新一轮刷新，同一个账号会被重复点。
    let now = chrono::Utc::now().timestamp();
    let due: Vec<String> = {
        let state = app.state::<AppState>();
        let mut cd = state.warmup_cooldown.lock().unwrap();
        targets
            .into_iter()
            .filter(|id| {
                let last = cd.get(id).copied().unwrap_or(0);
                if now - last < WARMUP_COOLDOWN_SECS {
                    false
                } else {
                    cd.insert(id.clone(), now);
                    true
                }
            })
            .collect()
    };

    if due.is_empty() {
        return;
    }

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // 串行执行：一次只起一个 CLI 进程，别把机器打满
        for id in due {
            match warmup_account_inner(&app_handle, &id).await {
                Ok(o) => eprintln!("[codex-helper] 自动激活「{}」成功", o.name),
                Err(e) => eprintln!("[codex-helper] 自动激活失败（{id}）：{e}"),
            }
        }
        let _ = app_handle.emit("warmup-finished", ());
    });
}

/// 切换账号的核心实现。
///
/// **顺序是刻意安排的，别随手改**：
///
/// 1. `restart_codex` 为真时，**先关掉 Codex 并等它退干净**。
///    Codex 在退出阶段有机会把内存里的旧 auth **回写**磁盘，
///    所以「先写文件再关进程」等于白写。
/// 2. 备份 → 写 auth.json → 按需合并 config.toml。
///    任何一步失败立刻中断，不会留下「auth 换了但 config 没换」的半成品。
/// 3. **最后把 Codex 重新拉起来**，让它读进新账号。
pub fn switch_account_inner(
    app: &AppHandle,
    id: &str,
    restart_codex: bool,
) -> Result<SwitchOutcome, String> {
    let (auth, cc_id, name, sync_cc, snippet) = snapshot(app, |vault| {
        vault
            .find(id)
            .map(|acc| {
                (
                    acc.auth.clone(),
                    acc.cc_id.clone(),
                    acc.name.clone(),
                    vault.settings.sync_ccswitch,
                    acc.config.clone(),
                )
            })
            .ok_or_else(|| "账号不存在".to_string())
    })?;

    // ---- 1) 先关 Codex -------------------------------------------------

    let mut codex_killed = 0usize;
    let mut app_exe: Option<PathBuf> = None;
    let mut had_app = false;
    if restart_codex {
        let targets = codexapp::find_running();
        // 桌面应用本体开着才需要重启；只关掉了几个命令行工具的话，
        // 说明用户压根没开 GUI，那也就没什么可拉起来的
        had_app = targets.iter().any(|p| p.app);
        app_exe = targets
            .iter()
            .find(|p| p.app)
            .and_then(|p| p.exe.as_ref())
            .map(PathBuf::from);
        // 关不掉就直接中断：宁可没切成功，也不要切完了进程还握着旧账号
        codex_killed = codexapp::kill_all(&targets)?;
    }

    // ---- 2) 改磁盘上的凭证 ----------------------------------------------

    // 先把当前登录态备份下来，万一切错了可以手工还原
    let backup_path = codex::backup_auth().map(|p| p.display().to_string()).ok();

    codex::write_auth(&auth).map_err(|e| format!("写入 auth.json 失败：{e}"))?;

    // 该账号带了 config.toml 片段就合并进去（只覆盖片段里出现的键）
    let mut config_applied = false;
    let mut config_backup_path = None;
    if let Some(snippet) = snippet.filter(|s| !s.trim().is_empty()) {
        config_backup_path = codex::backup_config().map(|p| p.display().to_string()).ok();
        let current = codex::read_config();
        let merged = codex::merge_config(&current, &snippet)
            .map_err(|e| format!("合并 config.toml 失败，已中止切换：{e}"))?;
        codex::write_config_text(&merged).map_err(|e| format!("写入 config.toml 失败：{e}"))?;
        config_applied = true;
    }

    // 尽量让 cc-switch 的当前 Provider 跟我们对齐；失败不影响切换本身
    let mut cc_synced = false;
    if sync_cc {
        if let Some(cc_id) = &cc_id {
            cc_synced = ccswitch::set_current_provider(cc_id).is_ok();
        }
    }

    {
        let state = app.state::<AppState>();
        let mut vault = state.vault.lock().unwrap();
        vault.current_id = Some(id.to_string());
        persist(app, &vault)?;
    }

    // ---- 3) 把 Codex 拉起来 ---------------------------------------------

    let mut codex_restarted = false;
    let mut codex_restart_error = None;
    if restart_codex && had_app {
        match codexapp::restart(app_exe.as_deref()) {
            Ok(aumid) => {
                codex_restarted = true;
                eprintln!("[codex-helper] Codex 已重新拉起（{aumid}）");
            }
            Err(e) => {
                // 只有「重新打开」这一步失败：账号其实已经切好了，
                // 所以不当成整体失败，只把原因带回去让界面提示一句
                eprintln!("[codex-helper] 重新拉起 Codex 失败：{e}");
                codex_restart_error = Some(e);
            }
        }
    }

    // ---- 文案 -----------------------------------------------------------

    let mut message = format!("已切换到「{name}」");
    if config_applied {
        message.push_str("（已同步 config.toml）");
    }
    if !restart_codex {
        message.push_str("，重启 Codex 后生效");
    } else if codex_killed == 0 {
        message.push_str("；Codex 当前没在运行，无需重启");
    } else if codex_restarted {
        message.push_str(&format!("，已关闭 {codex_killed} 个 Codex 进程并重新打开"));
    } else {
        message.push_str(&format!(
            "，已关闭 {codex_killed} 个 Codex 进程，但重新打开失败，请手动启动"
        ));
    }

    Ok(SwitchOutcome {
        ok: true,
        message,
        backup_path,
        ccswitch_synced: cc_synced,
        config_applied,
        config_backup_path,
        codex_killed,
        codex_restarted,
        codex_restart_error,
    })
}

/// 从 cc-switch 导入的核心实现。
pub fn import_from_ccswitch_inner(app: &AppHandle) -> Result<ImportOutcome, String> {
    let providers = ccswitch::read_codex_providers().map_err(|e| format!("{e}"))?;

    let mut imported = 0usize;
    let mut skipped = 0usize;

    {
        let state = app.state::<AppState>();
        let mut vault = state.vault.lock().unwrap();

        for p in &providers {
            if vault.contains_auth(&p.auth) {
                skipped += 1;
                continue;
            }
            let mut acc = ccswitch::to_account(p);
            acc.sort_index = vault.accounts.len() as i32;
            vault.accounts.push(acc);
            imported += 1;
        }

        // 顺手把 cc-switch 里标记为「当前」的那个也设成当前
        if imported > 0 {
            if let Some(cur) = providers.iter().find(|p| p.is_current) {
                if let Some(acc) = vault
                    .accounts
                    .iter()
                    .find(|a| a.cc_id.as_deref() == Some(cur.id.as_str()))
                {
                    vault.current_id = Some(acc.id.clone());
                }
            }
        }

        persist(app, &vault)?;
    }

    Ok(ImportOutcome {
        imported,
        skipped,
        message: format!("导入 {imported} 个，跳过重复 {skipped} 个"),
    })
}

/// 启动时用当前 auth.json 校准「当前账号」是哪一个。
pub fn sync_current_from_disk(app: &AppHandle) {
    let Ok(auth) = codex::read_auth() else { return };
    let Some(fp) = auth_fingerprint(&auth) else { return };

    let state = app.state::<AppState>();
    let mut vault = state.vault.lock().unwrap();
    let found = vault
        .accounts
        .iter()
        .find(|a| auth_fingerprint(&a.auth).as_deref() == Some(fp.as_str()))
        .map(|a| a.id.clone());

    if vault.current_id != found {
        vault.current_id = found;
        let _ = store::save(&vault);
    }
}

// ---------------------------------------------------------------- 命令

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Vec<AccountView> {
    to_views(&state.vault.lock().unwrap())
}

#[tauri::command]
pub async fn refresh_quotas(
    app: AppHandle,
    ids: Option<Vec<String>>,
) -> Result<Vec<AccountView>, String> {
    refresh_quotas_inner(&app, ids).await
}

/// 切换账号。
///
/// `restart_codex` 为真（默认）时，会先关掉 Codex 再切、切完重新拉起来。
/// 关进程要等它退干净，最坏能卡十几秒，所以整个流程丢到阻塞线程池跑，
/// 不能占着 async 运行时，更不能卡住主线程。
#[tauri::command]
pub async fn switch_account(
    app: AppHandle,
    id: String,
    restart_codex: Option<bool>,
) -> Result<SwitchOutcome, String> {
    let handle = app.clone();
    let restart = restart_codex.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || switch_account_inner(&handle, &id, restart))
        .await
        .map_err(|e| format!("切换任务执行失败：{e}"))?
}

/// 看看当前有哪些 Codex 进程会被关掉，给切换前的确认框显示用。
///
/// 只读，不动任何进程。扫进程表也就几毫秒，但仍然丢进阻塞线程池，保持口径统一。
#[tauri::command]
pub async fn codex_app_status() -> Result<codexapp::AppStatus, String> {
    tauri::async_runtime::spawn_blocking(codexapp::status)
        .await
        .map_err(|e| format!("查询 Codex 进程失败：{e}"))
}

#[tauri::command]
pub fn import_from_ccswitch(app: AppHandle) -> Result<ImportOutcome, String> {
    import_from_ccswitch_inner(&app)
}

#[tauri::command]
pub fn delete_account(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut vault = state.vault.lock().unwrap();
    vault.accounts.retain(|a| a.id != id);
    vault.quota_cache.remove(&id);
    if vault.current_id.as_deref() == Some(id.as_str()) {
        vault.current_id = None;
    }
    persist(&app, &vault)
}

#[tauri::command]
pub fn update_account(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    hidden: Option<bool>,
    sort_index: Option<i32>,
) -> Result<(), String> {
    let mut vault = state.vault.lock().unwrap();
    let acc = vault.find_mut(&id).ok_or_else(|| "账号不存在".to_string())?;
    if let Some(n) = name {
        acc.name = n;
    }
    if let Some(h) = hidden {
        acc.hidden = h;
    }
    if let Some(s) = sort_index {
        acc.sort_index = s;
    }
    persist(&app, &vault)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.vault.lock().unwrap().settings.clone()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut vault = state.vault.lock().unwrap();
        vault.settings = settings.clone();
        store::save(&vault).map_err(|e| format!("{e}"))?;
    }
    crate::apply_settings(&app);
    let _ = app.emit("settings-changed", ());
    Ok(())
}

#[tauri::command]
pub fn get_paths() -> serde_json::Value {
    serde_json::json!({
        "data_dir": store::data_dir().display().to_string(),
        "codex_home": codex::codex_home().display().to_string(),
        "auth_path": codex::auth_path().display().to_string(),
        "ccswitch_db": ccswitch::db_path().display().to_string(),
        "ccswitch_available": ccswitch::db_path().exists(),
    })
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = store::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{e}"))?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("{e}"))
}

// ---------------------------------------------------------------- 手动添加 / 编辑账号

/// 用户挑中的授权文件。
#[derive(Debug, Serialize)]
pub struct PickedFile {
    pub path: String,
    pub content: String,
}

/// 弹出系统文件选择框，读取一份授权文件。
#[tauri::command]
pub async fn pick_auth_file(app: AppHandle, title: Option<String>) -> Result<Option<PickedFile>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(title.unwrap_or_else(|| "选择 Codex 授权文件".to_string()))
        .add_filter("授权文件", &["json"])
        .add_filter("所有文件", &["*"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });

    let picked = rx.await.map_err(|e| format!("文件选择被中断：{e}"))?;
    let Some(file_path) = picked else {
        return Ok(None); // 用户取消
    };

    let path = file_path.into_path().map_err(|e| format!("{e}"))?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取文件失败：{e}"))?;

    Ok(Some(PickedFile {
        path: path.display().to_string(),
        content,
    }))
}

/// 校验并预览一段 auth JSON（前端边输入边调）。
#[tauri::command]
pub fn preview_auth(text: String) -> accounts::AuthPreview {
    match accounts::parse_auth_text(&text) {
        Ok(value) => accounts::preview(&value),
        Err(e) => accounts::AuthPreview {
            ok: false,
            error: Some(format!("{e}")),
            kind: None,
            email: None,
            account_id: None,
            credential: None,
        },
    }
}

/// 读取当前 `~/.codex/auth.json` 的原文，供「用当前登录态填充」使用。
#[tauri::command]
pub fn read_current_auth_text() -> Result<String, String> {
    let auth = codex::read_auth().map_err(|e| format!("{e}"))?;
    serde_json::to_string_pretty(&auth).map_err(|e| format!("{e}"))
}

/// 读取当前 `~/.codex/config.toml` 的原文，供「以当前配置为模板」使用。
#[tauri::command]
pub fn read_current_config_text() -> String {
    codex::read_config()
}

/// 编辑账号弹窗的回填数据。
#[derive(Debug, Serialize)]
pub struct AccountCredentials {
    pub name: String,
    /// auth.json 原文（已格式化），供编辑框回填
    pub auth: String,
    /// config.toml 片段，可能为空
    pub config: Option<String>,
}

/// 读取指定账号存着的授权内容，供「编辑授权」弹窗回填。
///
/// 注意：这里是唯一会把凭证送进 WebView 的入口，且只针对用户主动点开编辑的
/// 单个账号；列表接口 `list_accounts` 依旧不带任何凭证。
#[tauri::command]
pub fn read_account_credentials(
    state: State<'_, AppState>,
    id: String,
) -> Result<AccountCredentials, String> {
    let vault = state.vault.lock().unwrap();
    let acc = vault.find(&id).ok_or_else(|| "账号不存在".to_string())?;
    Ok(AccountCredentials {
        name: acc.name.clone(),
        auth: serde_json::to_string_pretty(&acc.auth).map_err(|e| format!("{e}"))?,
        config: acc.config.clone(),
    })
}

/// 手动创建一个账号。
#[tauri::command]
pub fn create_account(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: accounts::NewAccountPayload,
) -> Result<AccountView, String> {
    let id = uuid::Uuid::new_v4().to_string();

    let mut vault = state.vault.lock().unwrap();
    let sort_index = vault.accounts.len() as i32;
    let account = accounts::build_account(&payload, sort_index, id.clone())
        .map_err(|e| format!("{e}"))?;

    if vault.contains_auth(&account.auth) {
        return Err("这个账号已经在列表里了".to_string());
    }

    vault.accounts.push(account);
    persist(&app, &vault)?;

    Ok(to_views(&vault)
        .into_iter()
        .find(|v| v.id == id)
        .expect("刚插入的账号必然存在"))
}

/// 编辑账号的授权内容。
#[derive(Debug, Deserialize)]
pub struct EditAccountPayload {
    /// 传了就替换 auth（原文 JSON）
    #[serde(default)]
    pub auth: Option<String>,
    /// 传了就替换 config 片段；空串表示清空
    #[serde(default)]
    pub config: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// 修改已有账号的备注名 / 授权文件 / 配置片段。
#[tauri::command]
pub fn update_account_credentials(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    payload: EditAccountPayload,
) -> Result<AccountView, String> {
    // 1) 先把校验和新值全部算出来，这一步不碰账号库
    let parsed_auth = match payload.auth.as_deref() {
        Some(text) => {
            let value = accounts::parse_auth_text(text).map_err(|e| format!("{e}"))?;
            if let Some(err) = accounts::preview(&value).error {
                return Err(err);
            }
            Some(value)
        }
        None => None,
    };

    let incoming_config = payload
        .config
        .as_deref()
        .map(|c| c.trim().to_string());
    if let Some(c) = &incoming_config {
        if !c.is_empty() {
            accounts::validate_config_snippet(c).map_err(|e| format!("{e}"))?;
        }
    }

    // 2) 再落库
    {
        let mut vault = state.vault.lock().unwrap();
        let acc = vault
            .find_mut(&id)
            .ok_or_else(|| "账号不存在".to_string())?;

        if let Some(auth) = parsed_auth {
            acc.auth = auth;
            acc.email = codex::email_from_auth(&acc.auth);
            acc.account_id = codex::account_id_from_auth(&acc.auth);
            acc.kind = accounts::classify(&acc.auth);
            // 凭证换了，之前的套餐 / 额度缓存作废
            acc.plan_type = None;
        }

        if let Some(c) = incoming_config {
            acc.config = if c.is_empty() { None } else { Some(c) };
        }

        if let Some(n) = payload.name {
            let n = n.trim().to_string();
            if !n.is_empty() {
                acc.name = n;
            }
        }

        vault.quota_cache.remove(&id);
        persist(&app, &vault)?;
    }

    let vault = state.vault.lock().unwrap();
    to_views(&vault)
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| "账号不存在".to_string())
}

// ---------------------------------------------------------------- 激活 5 小时窗口

/// 激活某个账号的 5 小时额度窗口。
///
/// 做法：在**隔离的临时 CODEX_HOME** 里用 codex CLI 发一条极简会话，
/// 把窗口「点着」，之后 5 小时一到就会重置。
/// 全程不碰用户的 `~/.codex/auth.json`，所以也不会影响当前登录态。
pub async fn warmup_account_inner(
    app: &AppHandle,
    id: &str,
) -> Result<warmup::WarmupOutcome, String> {
    // 1) 取快照。这里是短锁，且**不能跨 await**
    let (auth, name) = snapshot(app, |vault| {
        vault
            .find(id)
            .map(|a| (a.auth.clone(), a.name.clone()))
            .ok_or_else(|| "账号不存在".to_string())
    })?;

    if !codex::is_official_auth(&auth) {
        return Err("只有官方账号才有 5 小时额度窗口".to_string());
    }

    // 2) 拉起 CLI。要十几秒，绝对不持锁
    let tail = warmup::warmup(&auth).await.map_err(|e| format!("{e}"))?;

    Ok(warmup::WarmupOutcome {
        ok: true,
        message: format!("已为「{name}」启动 5 小时窗口，约 5 小时后重置"),
        account_id: id.to_string(),
        name,
        log_tail: Some(tail),
    })
}

/// 把所有「5 小时窗口尚未启动」的账号一次性点着。
///
/// 串行执行 —— 一次只起一个 CLI 进程，既不给机器压力，也不像并发请求那么扎眼。
pub async fn warmup_all_dormant_inner(
    app: &AppHandle,
) -> Result<Vec<warmup::WarmupOutcome>, String> {
    let targets: Vec<(String, String)> = snapshot(app, |vault| {
        warmup::dormant_ids(vault)
            .into_iter()
            .filter_map(|id| vault.find(&id).map(|a| (id, a.name.clone())))
            .collect()
    });

    if targets.is_empty() {
        return Ok(Vec::new());
    }

    // 手动点过也算数，同样记进冷却，
    // 免得刚点完自动模式又对同一批账号重复发请求
    {
        let now = chrono::Utc::now().timestamp();
        let state = app.state::<AppState>();
        let mut cd = state.warmup_cooldown.lock().unwrap();
        for (id, _) in &targets {
            cd.insert(id.clone(), now);
        }
    }

    let mut out = Vec::with_capacity(targets.len());
    for (id, name) in targets {
        match warmup_account_inner(app, &id).await {
            Ok(o) => out.push(o),
            Err(e) => out.push(warmup::WarmupOutcome {
                ok: false,
                message: e,
                account_id: id,
                name,
                log_tail: None,
            }),
        }
    }

    Ok(out)
}

/// 手动激活单个账号（卡片上的按钮）。
#[tauri::command]
pub async fn warmup_account(app: AppHandle, id: String) -> Result<warmup::WarmupOutcome, String> {
    warmup_account_inner(&app, &id).await
}

/// 一键激活全部未启动的窗口。
#[tauri::command]
pub async fn warmup_all_dormant(app: AppHandle) -> Result<Vec<warmup::WarmupOutcome>, String> {
    warmup_all_dormant_inner(&app).await
}

/// 查一下 codex CLI 在哪，给设置页显示用（找不到返回 null）。
#[tauri::command]
pub fn codex_cli_path() -> Option<String> {
    warmup::find_cli().ok().map(|p| p.display().to_string())
}

// ---------------------------------------------------------------- token 统计

/// 按天统计 token 消耗。
///
/// 数据来自 `~/.codex` 下的 rollout 会话文件（当前 + 已归档），
/// 统计口径见 `stats` 模块头部注释。
///
/// 扫文件 + 解析 JSON 是纯 I/O 和 CPU 的活，丢到阻塞线程池去跑，
/// 免得把 async 运行时的工作线程占住。
#[tauri::command]
pub async fn token_stats(days: Option<u32>) -> Result<stats::TokenStats, String> {
    tauri::async_runtime::spawn_blocking(move || stats::collect(days))
        .await
        .map_err(|e| format!("统计任务执行失败：{e}"))
}
