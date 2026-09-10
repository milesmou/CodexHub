//! 按天统计 token 消耗。
//!
//! ## 数据来源
//! Codex 把每次会话逐条写进 rollout 文件（JSONL，一行一个事件）：
//! - `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` —— 当前会话
//! - `~/.codex/archived_sessions/rollout-*.jsonl`   —— 已归档会话
//!
//! ## 统计口径（重要，别改错）
//!
//! 直接把这些文件里的 token 字段加起来会错好几倍，原因有二：
//!
//! **一、同一个 payload 里有三个不同的用量字段，只有一个能用**
//!
//! `token_usage_record` 的 payload 里同时存在：
//! - `usage`              → **这一次 API 调用**的用量（增量）← 要的就是它
//! - `turn_token_usage`   → 本轮（一次用户提问）的累计，随调用次数滚动增长
//! - `thread_token_usage` → 整条会话的累计，等于把上下文重复投喂的部分反复计入
//!
//! 所以只有 `usage` 是真实消耗，另外两个拿来做「会话总量」展示可以，求和必然翻倍。
//!
//! **二、同一批数据可能同时躺在两个目录里**
//!
//! 会话被归档时，`sessions/` 下的那份可能还在。因此必须按
//! `payload.response_id`（每次 API 调用唯一）**全局去重**，不能按文件去重。
//!
//! **三、新旧两代格式并存，混着用会把一次调用算两遍**
//!
//! 老版本写的是 `event_msg` / `payload.type == "token_count"`，单次增量在
//! `payload.info.last_token_usage`。它和新格式的 `usage` 是**同一次调用**的两种记法。
//! 因此：一个文件里只要出现过 `token_usage_record`，就完全忽略它的 `event_msg`；
//! 只有整份文件都是旧格式时，才拿 `last_token_usage` 兜底。
//!
//! ## 时间
//! 取记录自带的 `timestamp`（RFC3339 / UTC），换算到**本地时区**后按自然日归档 ——
//! 用户看到的「今天」是他本地的今天，不是 UTC 的。

use crate::store;
use chrono::{DateTime, Local, NaiveDate};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// 一次 API 调用的原始用量（对应 rollout 里的 usage 对象）。
#[derive(Debug, Default, Clone, Copy)]
struct RawUsage {
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
    total: u64,
}

/// 从 JSON 的 usage / last_token_usage 对象里抠出各字段。
fn parse_usage(v: &serde_json::Value) -> RawUsage {
    let g = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    RawUsage {
        input: g("input_tokens"),
        cached: g("cached_input_tokens"),
        output: g("output_tokens"),
        reasoning: g("reasoning_output_tokens"),
        total: g("total_tokens"),
    }
}

/// 从一份 rollout 文件里抽出的一次调用记录。
struct Call {
    /// 本地自然日
    date: NaiveDate,
    usage: RawUsage,
    /// 全局去重键：新格式用 response_id，旧格式用「会话+时间+总量」兜底
    key: String,
    /// 会话 id，用来统计「涉及多少条会话」
    thread: String,
    /// 该轮用的模型（从 turn_context 里蹭到的，可能为空）
    model: String,
}

/// 内部累计器（不出参，用于按天 / 按模型分组）。
#[derive(Default)]
struct Acc {
    total: u64,
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
    calls: u64,
    threads: HashSet<String>,
}

impl Acc {
    fn add(&mut self, c: &Call) {
        self.total += c.usage.total;
        self.input += c.usage.input;
        self.cached += c.usage.cached;
        self.output += c.usage.output;
        self.reasoning += c.usage.reasoning;
        self.calls += 1;
        if !c.thread.is_empty() {
            self.threads.insert(c.thread.clone());
        }
    }
}

// ---------------------------------------------------------------- 出参

/// 某一天的用量。
#[derive(Debug, Serialize)]
pub struct DayStat {
    /// 本地日期，`YYYY-MM-DD`
    pub date: String,
    pub total: u64,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub reasoning: u64,
    /// 当天的 API 调用次数
    pub calls: u64,
    /// 当天涉及多少条会话
    pub threads: usize,
}

/// 某个模型的总用量。
#[derive(Debug, Serialize)]
pub struct ModelStat {
    pub model: String,
    pub total: u64,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub calls: u64,
}

/// 整个统计结果。
#[derive(Debug, Serialize)]
pub struct TokenStats {
    /// 按日期**升序**排列，只含真正有消耗的天
    pub days: Vec<DayStat>,
    /// 按总量降序的模型用量
    pub models: Vec<ModelStat>,
    pub total: u64,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub reasoning: u64,
    pub calls: u64,
    pub threads: usize,
    /// 扫过的会话文件数
    pub files: usize,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
    /// 口径说明，界面直接展示 —— 数字对不上时不用靠猜
    pub note: String,
}

const NOTE: &str =
    "按每次 API 调用的实际增量统计（token_usage_record.usage），已按 response_id 全局去重；\
     同时存在的旧格式 event_msg 记录已忽略，不会重复计入。时间按本地时区归入自然日。";

// ---------------------------------------------------------------- 扫描

/// `~/.codex` 下的 sessions / archived_sessions 两个目录。
fn session_dirs() -> Vec<PathBuf> {
    let home = store::home_dir();
    vec![
        home.join(".codex").join("sessions"),
        home.join(".codex").join("archived_sessions"),
    ]
}

/// 递归收集目录下所有 `.jsonl`。
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

/// 把 RFC3339 的 UTC 时间戳换算成本地自然日。
fn local_date(ts: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Local).date_naive())
}

/// 解析一份 rollout 文件，产出它贡献的调用记录。
///
/// 返回 `(调用列表, 本文件里已按 response_id 去重过的记录数)`。
fn parse_file(path: &Path) -> Vec<Call> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::with_capacity(256 * 1024, file);

    // 新格式记录
    let mut new_calls: Vec<Call> = Vec::new();
    // 旧格式记录（仅当本文件没有新格式时才采用）
    let mut old_calls: Vec<Call> = Vec::new();
    // turn_id -> 模型名，用来给用量打上模型标签
    let mut turn_model: HashMap<String, String> = HashMap::new();
    // 本文件内部的去重（跨文件的那道在聚合阶段统一做）
    let mut seen: HashSet<String> = HashSet::new();

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        match obj.get("type").and_then(|t| t.as_str()) {
            // 轮次上下文：顺手记下这一轮用的模型
            Some("turn_context") => {
                if let Some(p) = obj.get("payload") {
                    let tid = p.get("turn_id").and_then(|x| x.as_str()).unwrap_or("");
                    let model = p.get("model").and_then(|x| x.as_str()).unwrap_or("");
                    if !tid.is_empty() && !model.is_empty() {
                        turn_model.insert(tid.to_string(), model.to_string());
                    }
                }
            }
            // 新格式：单次调用的真实增量
            Some("token_usage_record") => {
                let Some(p) = obj.get("payload") else { continue };
                let Some(u) = p.get("usage") else { continue };
                let Some(date) = local_date(
                    obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or(""),
                ) else {
                    continue;
                };
                let usage = parse_usage(u);
                if usage.total == 0 && usage.input == 0 {
                    continue;
                }
                let rid = p.get("response_id").and_then(|x| x.as_str()).unwrap_or("");
                if !rid.is_empty() && !seen.insert(rid.to_string()) {
                    continue; // 同一文件里的重复行
                }
                let thread = p
                    .get("thread_id")
                    .or_else(|| p.get("session_id"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let turn = p.get("turn_id").and_then(|x| x.as_str()).unwrap_or("");
                let model = turn_model.get(turn).cloned().unwrap_or_default();
                let key = if rid.is_empty() {
                    format!("{thread}|{}|{}", obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or(""), usage.total)
                } else {
                    rid.to_string()
                };
                new_calls.push(Call {
                    date,
                    usage,
                    key,
                    thread,
                    model,
                });
            }
            // 旧格式：只有整份文件都没有新格式记录时才用它
            Some("event_msg") => {
                let Some(p) = obj.get("payload") else { continue };
                if p.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                    continue;
                }
                let Some(last) = p.get("info").and_then(|i| i.get("last_token_usage")) else {
                    continue;
                };
                let Some(date) = local_date(
                    obj.get("timestamp").and_then(|t| t.as_str()).unwrap_or(""),
                ) else {
                    continue;
                };
                let usage = parse_usage(last);
                if usage.total == 0 && usage.input == 0 {
                    continue;
                }
                let thread = p
                    .get("thread_id")
                    .or_else(|| p.get("session_id"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let ts = obj
                    .get("timestamp")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let key = format!("{thread}|{ts}|{}", usage.total);
                old_calls.push(Call {
                    date,
                    usage,
                    key,
                    thread,
                    model: String::new(),
                });
            }
            _ => {}
        }
    }

    // 有新格式就只认新格式，避免和 event_msg 把同一次调用算两遍
    if new_calls.is_empty() {
        old_calls
    } else {
        new_calls
    }
}

/// 扫描全部会话文件，按天 + 按模型汇总 token 消耗。
///
/// `days`：只看最近多少天（`None` = 全部）。
pub fn collect(days: Option<u32>) -> TokenStats {
    let mut files_in: Vec<PathBuf> = Vec::new();
    for dir in session_dirs() {
        collect_jsonl(&dir, &mut files_in);
    }
    let file_count = files_in.len();

    // 汇聚所有文件，然后全局去重
    let mut all: Vec<Call> = Vec::new();
    for path in &files_in {
        all.extend(parse_file(path));
    }

    let mut global_seen: HashSet<String> = HashSet::new();
    let mut by_day: BTreeMap<NaiveDate, Acc> = BTreeMap::new();
    let mut by_model: HashMap<String, Acc> = HashMap::new();
    let mut overall = Acc::default();

    // 时间下界：只看最近 N 天
    let cutoff = days.filter(|d| *d > 0).map(|d| {
        Local::now().date_naive() - chrono::Duration::days(d as i64 - 1)
    });

    for c in &all {
        if !global_seen.insert(c.key.clone()) {
            continue; // 同一批数据在两个目录里各有一份
        }
        if let Some(cut) = cutoff {
            if c.date < cut {
                continue;
            }
        }
        by_day.entry(c.date).or_default().add(c);
        overall.add(c);
        if !c.model.is_empty() {
            by_model.entry(c.model.clone()).or_default().add(c);
        }
    }

    let mut days: Vec<DayStat> = by_day
        .into_iter()
        .map(|(date, a)| DayStat {
            date: date.format("%Y-%m-%d").to_string(),
            total: a.total,
            input: a.input,
            cached: a.cached,
            output: a.output,
            reasoning: a.reasoning,
            calls: a.calls,
            threads: a.threads.len(),
        })
        .collect();
    days.sort_by(|a, b| a.date.cmp(&b.date));

    let mut models: Vec<ModelStat> = by_model
        .into_iter()
        .map(|(model, a)| ModelStat {
            model,
            total: a.total,
            input: a.input,
            cached: a.cached,
            output: a.output,
            calls: a.calls,
        })
        .collect();
    models.sort_by(|a, b| b.total.cmp(&a.total));

    TokenStats {
        first_day: days.first().map(|d| d.date.clone()),
        last_day: days.last().map(|d| d.date.clone()),
        days,
        models,
        total: overall.total,
        input: overall.input,
        cached: overall.cached,
        output: overall.output,
        reasoning: overall.reasoning,
        calls: overall.calls,
        threads: overall.threads.len(),
        files: file_count,
        note: NOTE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_usage_reads_all_fields() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"input_tokens":10,"cached_input_tokens":8,"output_tokens":2,
                "reasoning_output_tokens":1,"total_tokens":12}"#,
        )
        .unwrap();
        let u = parse_usage(&v);
        assert_eq!(u.input, 10);
        assert_eq!(u.cached, 8);
        assert_eq!(u.output, 2);
        assert_eq!(u.reasoning, 1);
        assert_eq!(u.total, 12);
    }

    #[test]
    fn parse_usage_tolerates_missing_fields() {
        let v: serde_json::Value = serde_json::from_str(r#"{"total_tokens":5}"#).unwrap();
        let u = parse_usage(&v);
        assert_eq!(u.total, 5);
        assert_eq!(u.input, 0);
    }

    #[test]
    fn local_date_parses_rfc3339() {
        assert!(local_date("2026-09-10T07:36:31.111Z").is_some());
        assert!(local_date("not-a-date").is_none());
    }
}
