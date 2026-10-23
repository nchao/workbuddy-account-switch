//! core.rs —— 档案（登录态快照）的读写逻辑
//!
//! 与命令行版 wbswitch 共用同一份数据格式：
//!   ~/.workbuddy-account-switch/profiles/<档案名>/{files/, manifest.json}

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn home() -> PathBuf {
    dirs_path()
}

fn dirs_path() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn wb_dir() -> PathBuf {
    home().join(".workbuddy")
}

/// WorkBuddy daemon 真正的登录凭证目录（在 ~/.workbuddy 之外）。
/// 里面的 workbuddy-desktop.info 是完整会话（access token 等）；
/// 在 App 里点「退出账号」会把它改名备份、并写入 .logged-out 标记——
/// 标记存在时启动会直接忽略凭证文件（日志里的 logoutMarkerHit）。
pub fn ext_auth_dir() -> PathBuf {
    home().join("Library/Application Support/CodeBuddyExtension/Data/Public/auth")
}

pub fn store_dir() -> PathBuf {
    home().join(".workbuddy-account-switch")
}

pub fn profiles_dir() -> PathBuf {
    store_dir().join("profiles")
}

pub fn backups_dir() -> PathBuf {
    store_dir().join("backups")
}

/// 构成「一次登录完整状态」的相对路径（相对 ~/.workbuddy）
pub const AUTH_ITEMS: [&str; 4] = [
    "app/session/Cookies",
    "app/session/Local Storage",
    "app/session/Session Storage",
    "app/session/WebStorage",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    // 字段名与命令行版 wbswitch（Python）保持一致；老档案里的 note 字段会被自动忽略
    #[serde(default, rename = "createdAt")]
    pub created_at: String,
    #[serde(default, rename = "createdTs")]
    pub created_ts: u64,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub uid: Option<String>,
    /// 快照里 session cookie 的长度，0/None 说明这份存档没抓到登录态
    #[serde(default, rename = "sessionLen")]
    pub session_len: Option<usize>,
    /// 账号快照的可读标签，例如「张三（某某公司·ultimate）」
    #[serde(default, rename = "accountLabel")]
    pub account_label: Option<String>,
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub created_at: String,
    pub uid: Option<String>,
    pub fingerprint: Option<String>,
    pub session_len: Option<usize>,
    pub account_label: Option<String>,
    pub path: PathBuf,
    pub size_kb: u64,
    pub is_current: bool,
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '(' | ')' | '（' | '）' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.trim().to_string()
}

/// 有些账号的昵称被存成一串「区域指示符」emoji（🇦 🇱 🇮 🇨 🇪 其实就是 ALICE），
/// 直接拿来当档案名又丑又难输入，这里还原成普通字母
fn normalize_nickname(s: &str) -> String {
    let mut out = String::new();
    let mut decoded = false;
    for c in s.chars() {
        let cp = c as u32;
        if (0x1F1E6..=0x1F1FF).contains(&cp) {
            out.push((b'A' + (cp - 0x1F1E6) as u8) as char);
            decoded = true;
        } else if c.is_whitespace() || c == '\u{200b}' || c == '\u{fe0f}' {
            // 指示符之间的分隔符 / 变体选择符，丢掉
        } else {
            out.push(c);
        }
    }
    if decoded {
        out
    } else {
        s.to_string()
    }
}

/// 保存时的建议名称：从账号快照里取「昵称(企业名)」，如 张三(某某公司)
pub fn suggested_name() -> String {
    let f = wb_dir().join("storage/skeleton/account-snapshot.json");
    if let Ok(text) = fs::read_to_string(&f) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(p) = v.get("primary") {
                let nick = normalize_nickname(
                    p.get("nickname").and_then(|x| x.as_str()).unwrap_or(""),
                );
                let ent = p.get("enterpriseName").and_then(|x| x.as_str()).unwrap_or("");
                let typ = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                if !nick.is_empty() {
                    return if !ent.is_empty() {
                        format!("{}({})", nick, ent)
                    } else if typ == "personal" {
                        format!("{}(个人版)", nick)
                    } else {
                        nick.to_string()
                    };
                }
            }
        }
    }
    // 兜底：账号N（按现有档案数+1）
    let n = list_profiles().len() + 1;
    format!("账号{}", n)
}

/// 本地时区相对 UTC 的偏移秒数（读不到就按东八区算）
fn local_offset_secs() -> i64 {
    if let Ok(out) = Command::new("/bin/date").arg("+%z").output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let b = s.as_bytes();
        if b.len() == 5 && (b[0] == b'+' || b[0] == b'-') {
            let h: i64 = s[1..3].parse().unwrap_or(0);
            let m: i64 = s[3..5].parse().unwrap_or(0);
            let sign = if b[0] == b'-' { -1 } else { 1 };
            return sign * (h * 3600 + m * 60);
        }
    }
    8 * 3600
}

fn now_string() -> String {
    // 本地时间字符串，格式与命令行版一致：YYYY-MM-DD HH:MM:SS
    let secs = (now_ts() as i64 + local_offset_secs()).max(0) as u64;
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    // Howard Hinnant 的 civil_from_days 算法
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as i64;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 文件系统

fn copy_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_all(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        let _ = fs::set_permissions(dst, fs::Permissions::from_mode(0o600));
        Ok(())
    }
}

/// 复制 Cookies（SQLite）：优先用 VACUUM INTO 做一致性导出，
/// 这样即使 WorkBuddy 正在运行，也不会抄到写了一半的数据库。
/// 导出后必须清掉旁边的 -journal / -wal，否则快照只读打开时会要求回滚而失败。
fn copy_cookies(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    clean_sqlite_sidecars(dst);
    let _ = fs::remove_file(dst);

    // 1) 把源库连同 -journal / -wal 一起复制到临时目录。
    //    绝不直接读写源库：WorkBuddy 正在用它，且只读打开带 hot journal 的库必然失败。
    let tmp_root = store_dir().join("tmp");
    let _ = fs::create_dir_all(&tmp_root);
    let t = tmp_root.join("Cookies");
    clean_sqlite_sidecars(&t);
    let _ = fs::remove_file(&t);
    let copied = fs::copy(src, &t).is_ok();
    if copied {
        let sbase = src.to_string_lossy().to_string();
        let tbase = t.to_string_lossy().to_string();
        for sfx in ["-journal", "-wal", "-shm"] {
            let s = format!("{}{}", sbase, sfx);
            if Path::new(&s).exists() {
                let _ = fs::copy(&s, format!("{}{}", tbase, sfx));
            }
        }
    }

    // 2) 在临时副本上以读写方式打开（自动回滚残留 journal），再一致性导出
    let target = dst.to_string_lossy().replace('\'', "''");
    let mut ok = false;
    if copied {
        if let Ok(conn) = rusqlite::Connection::open(&t) {
            ok = conn
                .execute_batch(&format!("VACUUM INTO '{}'", target))
                .is_ok();
        }
    }
    if !ok {
        // 退回：直接复制那份已经恢复过的临时副本
        let _ = fs::remove_file(dst);
        clean_sqlite_sidecars(dst);
        if copied {
            fs::copy(&t, dst)?;
        } else {
            copy_all(src, dst)?;
        }
    }
    if !sqlite_readable(dst) {
        // 最后兜底：逐行读出内容，重建一个干净的库
        let _ = fs::remove_file(dst);
        clean_sqlite_sidecars(dst);
        if !rebuild_cookies(&t, dst) {
            let _ = fs::remove_file(dst);
            clean_sqlite_sidecars(dst);
            copy_all(src, dst)?;
        }
    }
    clean_sqlite_sidecars(dst);
    let _ = fs::remove_dir_all(&tmp_root);
    let _ = fs::set_permissions(dst, fs::Permissions::from_mode(0o600));
    Ok(())
}

/// 从源库里把 meta / cookies 原样读出来，写进一个新建的干净 SQLite 文件
fn rebuild_cookies(src: &Path, dst: &Path) -> bool {
    // 读写打开（临时副本上的残留 journal 会被自动回滚），只读的话带 journal 会直接失败
    let src_conn = match rusqlite::Connection::open(src) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let dst_conn = match rusqlite::Connection::open(dst) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // 表结构照抄
    let mut ddl: Vec<String> = Vec::new();
    {
        let mut stmt = match src_conn.prepare(
            "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND type IN ('table','index')",
        ) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let rows = match stmt.query_map([], |r| r.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return false,
        };
        for sql in rows.flatten() {
            ddl.push(sql);
        }
    }
    if ddl.is_empty() {
        return false;
    }
    if dst_conn.execute_batch("PRAGMA journal_mode=DELETE;").is_err() {
        return false;
    }
    for sql in &ddl {
        let _ = dst_conn.execute(sql, []);
    }

    // 逐表复制数据
    let mut tables: Vec<String> = Vec::new();
    {
        let mut stmt = match src_conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        {
            Ok(s) => s,
            Err(_) => return false,
        };
        let rows = match stmt.query_map([], |r| r.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return false,
        };
        for t in rows.flatten() {
            tables.push(t);
        }
    }
    for t in &tables {
        let cols: Vec<String> = {
            let stmt = match src_conn.prepare(&format!("SELECT * FROM \"{}\" LIMIT 1", t)) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let names: Vec<String> = stmt
                .column_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            names
        };
        if cols.is_empty() {
            continue;
        }
        let col_list = cols
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = vec!["?"; cols.len()].join(",");
        let select = format!("SELECT {} FROM \"{}\"", col_list, t);
        let insert = format!("INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})", t, col_list, placeholders);

        let mut stmt = match src_conn.prepare(&select) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let col_count = cols.len();
        let mut rows = match stmt.query([]) {
            Ok(r) => r,
            Err(_) => continue,
        };
        loop {
            let row = match rows.next() {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(_) => break,
            };
            let mut values: Vec<rusqlite::types::Value> = Vec::new();
            for i in 0..col_count {
                values.push(match row.get::<_, rusqlite::types::Value>(i) {
                    Ok(v) => v,
                    Err(_) => rusqlite::types::Value::Null,
                });
            }
            let _ = dst_conn.execute(&insert, rusqlite::params_from_iter(values));
        }
    }
    // 落盘并清干净
    let _ = dst_conn.execute_batch("VACUUM;");
    drop(dst_conn);
    clean_sqlite_sidecars(dst);
    sqlite_readable(dst)
}

fn clean_sqlite_sidecars(p: &Path) {
    let base = p.to_string_lossy().to_string();
    for suffix in ["-journal", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", base, suffix));
    }
}

fn sqlite_readable(p: &Path) -> bool {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        p,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return false;
    };
    conn.query_row("SELECT count(*) FROM cookies", [], |r| r.get::<_, i64>(0))
        .is_ok()
}

/// 按相对路径复制：Cookies 走一致性导出，其余走普通复制
fn copy_item(rel: &str, src: &Path, dst: &Path) -> std::io::Result<()> {
    if rel == "app/session/Cookies" {
        copy_cookies(src, dst)
    } else {
        copy_all(src, dst)
    }
}

fn remove_all(p: &Path) -> std::io::Result<()> {
    if p.is_dir() {
        fs::remove_dir_all(p)
    } else if p.exists() {
        fs::remove_file(p)
    } else {
        Ok(())
    }
}

fn dir_size(p: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![p.to_path_buf()];
    while let Some(cur) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&cur) {
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(md) = fs::metadata(&path) {
                    total += md.len();
                }
            }
        }
    }
    total
}

// ---------------------------------------------------------------- 探测

/// 主进程是 `/Applications/WorkBuddy.app/Contents/MacOS/Electron`（不是 WorkBuddy）；
/// Helper / sandbox-center / node daemon 是常驻后台进程，不算「在运行」，
/// 否则 App 退出后工具会一直误判为运行中，切换流程永远等不到退出。
pub fn wb_running() -> bool {
    !wb_gui_pids().is_empty()
}

/// WorkBuddy 图形主进程的 pid（仅 Contents/MacOS/ 下的 Electron / WorkBuddy）
pub fn wb_gui_pids() -> Vec<i32> {
    wb_procs()
        .into_iter()
        .filter(|(_, cmd)| cmd.starts_with("/Applications/WorkBuddy.app/Contents/MacOS/"))
        .map(|(pid, _)| pid)
        .collect()
}

/// 所有 WorkBuddy 相关进程（主进程 / Helper / 常驻 daemon / node），
/// 排除切换器自身，用于切换前的清场强杀
pub fn wb_pids() -> Vec<i32> {
    wb_procs().into_iter().map(|(pid, _)| pid).collect()
}

fn wb_procs() -> Vec<(i32, String)> {
    let out = Command::new("/usr/bin/pgrep")
        .args(["-fl", "WorkBuddy"])
        .output();
    let Ok(out) = out else { return vec![] };
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.lines()
        .filter_map(|line| {
            if !line.contains("/Applications/WorkBuddy.app/")
                && !line.contains("/.workbuddy/binaries/")
            {
                return None;
            }
            if line.contains("wbswitch") || line.contains("账号切换") {
                return None;
            }
            let mut it = line.splitn(2, ' ');
            let pid = it.next()?.trim().parse::<i32>().ok()?;
            let cmd = it.next().unwrap_or("").trim().to_string();
            Some((pid, cmd))
        })
        .collect()
}

/// 切换 / 本地退出前的清场：强杀所有 WorkBuddy 相关进程（含常驻 daemon）。
/// daemon 持有登录凭证文件句柄，不杀掉可能把内存里的旧凭证写回磁盘。
pub fn kill_all_wb_procs() {
    for pid in wb_pids() {
        let _ = Command::new("/bin/kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
    if !wb_pids().is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
}

/// 请求 WorkBuddy 退出，并等待它（含 daemon sidecar）彻底退出
/// timeout_secs 后仍未退出则强杀，再等待
#[allow(dead_code)]
pub fn ensure_quit(timeout_secs: u64) -> Result<(), String> {
    if !wb_running() {
        return Ok(());
    }
    let _ = quit_workbuddy();
    let mut waited = 0u64;
    while waited < timeout_secs {
        std::thread::sleep(std::time::Duration::from_millis(500));
        waited += 1;
        if !wb_running() {
            return Ok(());
        }
    }
    // 仍未退出：强杀 WorkBuddy 进程组（不会误伤切换器自身）
    for pid in wb_pids() {
        let _ = Command::new("/bin/kill").args(["-9", &pid.to_string()]).status();
    }
    let mut waited2 = 0u64;
    while waited2 < 12 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        waited2 += 1;
        if !wb_running() {
            return Ok(());
        }
    }
    Err("WorkBuddy 仍未完全退出，请手动 Cmd+Q 后再切换".into())
}

/// 某个 Cookies 库对应的登录态指纹（session cookie 值的哈希，不解密、不输出原始凭据）
pub fn fingerprint_of(cookies: &Path) -> Option<String> {
    if !cookies.exists() {
        return None;
    }
    let mut payload: Vec<u8> = Vec::new();
    if let Ok(conn) = rusqlite::Connection::open_with_flags(
        &cookies,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT host_key, name, encrypted_value, value FROM cookies WHERE host_key LIKE '%workbuddy.cn%'",
        ) {
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    cell_bytes(r.get::<_, rusqlite::types::Value>(2)?),
                    cell_bytes(r.get::<_, rusqlite::types::Value>(3)?),
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let (host, name, enc, plain) = row;
                    if matches!(
                        name.as_str(),
                        "session" | "token" | "access_token" | "uid" | "refresh_token"
                    ) {
                        payload.extend_from_slice(host.as_bytes());
                        payload.extend_from_slice(name.as_bytes());
                        // cookie 明文值优先（本机的 session cookie 就是明文存的）
                        let v = if !plain.is_empty() { plain } else { enc };
                        payload.extend_from_slice(&v);
                    }
                }
            }
        }
    }
    if payload.is_empty() {
        return None;
    }
    let digest = Sha256::digest(&payload);
    Some(hex(&digest[..])[..16].to_string())
}

/// 当前（磁盘上）的登录态指纹。
/// 兜底链：主 cookie 库 → webview 分区 → daemon 凭证（个人账号只有这一处）
pub fn current_fingerprint() -> Option<String> {
    let main = wb_dir().join("app/session/Cookies");
    if let Some(fp) = fingerprint_of(&main) {
        return Some(fp);
    }
    let parts = wb_dir().join("app/session/Partitions");
    if let Ok(entries) = fs::read_dir(&parts) {
        for e in entries.flatten() {
            if let Some(fp) = fingerprint_of(&e.path().join("Cookies")) {
                return Some(fp);
            }
        }
    }
    current_daemon_cred().map(|c| daemon_fingerprint(&c))
}

/// 档案快照里的登录态指纹。
/// 命令行版（Python）当年算指纹时只取了 encrypted_value（本机为空），
/// 所以老档案的指纹跟真实票据无关；一律按快照库重算，保证「当前账号」判定准确。
pub fn snapshot_fingerprint(pdir: &Path) -> Option<String> {
    fingerprint_of(&pdir.join("files").join("app/session/Cookies"))
        .or_else(|| saved_daemon_cred(pdir).map(|c| daemon_fingerprint(&c)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 把 SQLite 里读到的单元格统一转成字节。
/// 注意：cookie 的 value 列在这台机器上是 TEXT，直接按 Vec<u8>（Blob）读会
/// 报 InvalidColumnType 导致整行被跳过 —— 必须先按 Value 取再转换。
fn cell_bytes(v: rusqlite::types::Value) -> Vec<u8> {
    use rusqlite::types::Value;
    match v {
        Value::Blob(b) => b,
        Value::Text(s) => s.into_bytes(),
        Value::Integer(i) => i.to_string().into_bytes(),
        Value::Real(f) => f.to_string().into_bytes(),
        Value::Null => Vec::new(),
    }
}

// ------------------------------------------------- daemon 凭证（第三级票据载体）
//
// 实锤（2026-10-23）：**个人账号的票据根本不在 webview cookie 库里**。
// 个人账号登录 ~/.workbuddy/app/session/Cookies 里只有 tgw_l7_route / dark_mode_setting，
// 没有任何 session cookie；票据只存在于 daemon 凭证 workbuddy-desktop.info
// （里面的 account.uid / account.type 是明文，token 是本机加密信封）。
// 企业账号则两处都有。所以票据检测必须做「主库 → 分区 → daemon」三级兜底。

/// 活凭证文件名判断：workbuddy-desktop.info 是活的；
/// 历史备份形如 workbuddy-desktop.2026-10-13T01-28-25-494Z.10787.<uuid>.info（多带几段后缀）
fn is_live_cred_name(name: &str) -> bool {
    name.ends_with(".info") && !name[..name.len() - 5].contains('.')
}

/// 在给定目录里挑一份活凭证（有多份时取最后修改的）
fn pick_live_cred(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, SystemTime)> = None;
    for e in fs::read_dir(dir).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !is_live_cred_name(&n) {
            continue;
        }
        // App 内点「退出账号」会留下同名 .logged-out 标记，有标记时凭证被直接忽略
        if dir.join(format!("{}.logged-out", n)).exists() {
            continue;
        }
        let m = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        if best.as_ref().map_or(true, |(_, t)| m > *t) {
            best = Some((e.path(), m));
        }
    }
    best.map(|(p, _)| p)
}

/// 当前生效的 daemon 凭证路径
pub fn live_cred_path() -> Option<PathBuf> {
    pick_live_cred(&ext_auth_dir())
}

/// 档案里保存的 daemon 凭证（files-ext/auth/）
fn saved_cred_path(pdir: &Path) -> Option<PathBuf> {
    pick_live_cred(&pdir.join("files-ext").join("auth"))
}

/// daemon 凭证里读出的账号身份（uid/type 是明文；token 只取长度，不解密不落盘）
#[derive(Debug, Clone, Default)]
pub struct DaemonCred {
    pub uid: String,
    pub typ: String,
    pub uin: String,
    /// 凭证文件字节数，用作「有没有票据 / 票据多大」的度量
    pub bytes: usize,
}

fn daemon_cred_of(path: &Path) -> Option<DaemonCred> {
    let text = fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let uid = v
        .get("account")
        .and_then(|a| a.get("uid"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if uid.is_empty() {
        return None;
    }
    Some(DaemonCred {
        uid,
        typ: v
            .get("account")
            .and_then(|a| a.get("type"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        // uin 可能是数字也可能是字符串，统一取不带引号的形式
        uin: v
            .get("account")
            .and_then(|a| a.get("uin"))
            .map(|x| match x {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        bytes: fs::metadata(path)
            .map(|m| m.len() as usize)
            .unwrap_or(text.len()),
    })
}

/// daemon 凭证指纹：只取账号身份（uid/type/uin），token 刷新后依然稳定
fn daemon_fingerprint(c: &DaemonCred) -> String {
    let payload = format!("wbdaemon|{}|{}|{}", c.uid, c.typ, c.uin);
    let digest = Sha256::digest(payload.as_bytes());
    hex(&digest[..])[..16].to_string()
}

/// 当前 daemon 凭证的账号身份（没有则 None）
pub fn current_daemon_cred() -> Option<DaemonCred> {
    daemon_cred_of(&live_cred_path()?)
}

/// 档案里 daemon 凭证的账号身份（没有则 None）
fn saved_daemon_cred(pdir: &Path) -> Option<DaemonCred> {
    daemon_cred_of(&saved_cred_path(pdir)?)
}

/// 启发式探测当前登录的 uid（最近写入的 startup-route.json 所在目录）
pub fn detect_uid() -> Option<String> {
    // 优先用 App 登录时写入的账号快照（最权威，切换后立刻反映新账号）
    let snap = wb_dir().join("storage/skeleton/account-snapshot.json");
    if let Ok(text) = fs::read_to_string(&snap) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(u) = v.get("primary").and_then(|p| p.get("uid")).and_then(|x| x.as_str())
            {
                if !u.is_empty() {
                    return Some(u.to_string());
                }
            }
        }
    }
    // 次选 daemon 凭证里的明文 uid（账号快照被清掉时仍能识别个人账号）
    if let Some(c) = current_daemon_cred() {
        return Some(c.uid);
    }
    let wb = wb_dir();
    let Ok(entries) = fs::read_dir(&wb) else {
        return None;
    };
    let mut best: Option<(String, u64)> = None;
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let uid = if name.starts_with("user-") {
            name[5..].split('-').next().unwrap_or("").to_string()
        } else {
            name.clone()
        };
        if uid.len() != 36 {
            continue;
        }
        for cand in ["startup-route.json", "sidebar-list-snapshot.json"] {
            let f = path.join(cand);
            if let Ok(md) = fs::metadata(&f) {
                if let Ok(t) = md.modified() {
                    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    if best.as_ref().map(|(_, ts)| secs > *ts).unwrap_or(true) {
                        best = Some((uid.clone(), secs));
                    }
                }
            }
        }
    }
    best.map(|(u, _)| u)
}

fn uid_dirs() -> Vec<(String, PathBuf)> {
    let mut res = Vec::new();
    let Ok(entries) = fs::read_dir(wb_dir()) else {
        return res;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let uid = if name.starts_with("user-") {
            name[5..].split('-').next().unwrap_or("").to_string()
        } else {
            name.clone()
        };
        if uid.len() == 36 && uid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            res.push((format!("{}/{}", name, name), path));
        }
    }
    res
}

fn snapshot_items() -> Vec<String> {
    let wb = wb_dir();
    let mut items: Vec<String> = AUTH_ITEMS
        .iter()
        .filter(|i| wb.join(i).exists())
        .map(|i| i.to_string())
        .collect();

    // per-user webview 分区（登出时会被 App 删除，必须一起备份）
    // 只取登录相关的子目录，跳过 Cache / GPUCache 等体积大的缓存
    let parts = wb.join("app/session/Partitions");
    if let Ok(entries) = fs::read_dir(&parts) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("wb-webview-") {
                continue;
            }
            for sub in ["Cookies", "Local Storage", "Session Storage"] {
                if e.path().join(sub).exists() {
                    items.push(format!("app/session/Partitions/{}/{}", name, sub));
                }
            }
        }
    }

    // 用户态存储 / 配置缓存 / 加密凭据目录 / 账号快照
    for extra in ["app/users", "app/users/", "local_storage", "security", "storage/skeleton"] {
        let p = extra.trim_end_matches('/');
        if wb.join(p).exists() {
            items.push(p.to_string());
        }
    }
    if let Ok(entries) = fs::read_dir(wb.join("storage")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && name.starts_with("user-") {
                items.push(format!("storage/{}", name));
            }
        }
    }

    // 按 uid 命名的状态目录
    for (rel, _) in uid_dirs() {
        items.push(rel.split('/').next().unwrap_or(&rel).to_string());
    }
    items.sort();
    items.dedup();
    items
}

// ---------------------------------------------------------------- 档案操作

pub fn list_profiles() -> Vec<Profile> {
    let cur_fp = current_fingerprint();
    let cur_uid = detect_uid();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(profiles_dir()) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.starts_with("auto-") {
            continue;
        }
        let man = read_manifest(&path);
        // 指纹与票据一律以快照库为准（兼容命令行版老档案）
        let fp = snapshot_fingerprint(&path).or_else(|| man.fingerprint.clone());
        let slen = man.session_len.or_else(|| snapshot_session_len(&path));
        // 「当前账号」判定：指纹相同，或者 uid 相同。
        // 加 uid 这一路是因为 token 刷新会让票据指纹漂移（企业账号尤其明显），
        // 只比指纹会出现「明明就是这个账号，却没标当前」的假阴性。
        let uid_hit = cur_uid
            .as_ref()
            .zip(man.uid.as_ref())
            .map(|(a, b)| a == b)
            .unwrap_or(false);
        out.push(Profile {
            is_current: uid_hit || (fp.is_some() && fp == cur_fp),
            name: man.name.clone(),
            created_at: man.created_at.clone(),
            uid: man.uid.clone(),
            fingerprint: fp,
            session_len: slen,
            account_label: man.account_label.clone().or_else(|| snapshot_account_label(&path)),
            size_kb: dir_size(&path) / 1024,
            path: path.clone(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn read_manifest(pdir: &Path) -> Manifest {
    let f = pdir.join("manifest.json");
    let default_name = pdir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if let Ok(text) = fs::read_to_string(&f) {
        if let Ok(m) = serde_json::from_str::<Manifest>(&text) {
            return m;
        }
    }
    Manifest {
        name: default_name,
        created_at: String::new(),
        created_ts: 0,
        fingerprint: None,
        uid: None,
        session_len: None,
        account_label: None,
        items: vec![],
    }
}

fn write_manifest(pdir: &Path, man: &Manifest) -> Result<(), String> {
    let text = serde_json::to_string_pretty(man).map_err(|e| e.to_string())?;
    fs::write(pdir.join("manifest.json"), text).map_err(|e| e.to_string())
}

/// 把当前登录态存进指定目录（`files/` + `manifest.json` 结构）
pub fn save_current_to(pdir: &Path, name: &str) -> Result<Profile, String> {
    if pdir.exists() {
        remove_all(pdir).map_err(|e| format!("清理旧档案失败：{}", e))?;
    }
    fs::create_dir_all(pdir.join("files")).map_err(|e| e.to_string())?;

    let items = snapshot_items();
    if items.is_empty() {
        return Err("没有找到可保存的登录态文件".into());
    }
    let wb = wb_dir();
    for rel in &items {
        let src = wb.join(rel);
        if src.exists() {
            copy_item(rel, &src, &pdir.join("files").join(rel))
                .map_err(|e| format!("复制 {} 失败：{}", rel, e))?;
        }
    }

    // daemon 凭证（在 ~/.workbuddy 之外，之前漏了它导致还原后仍显示未登录）
    save_ext_auth(pdir);

    let man = Manifest {
        name: name.to_string(),
        created_at: now_string(),
        created_ts: now_ts(),
        fingerprint: current_fingerprint(),
        uid: detect_uid(),
        items,
        session_len: None,
        account_label: None,
    };
    let man = Manifest {
        session_len: snapshot_session_len(pdir),
        account_label: snapshot_account_label(pdir),
        ..man
    };
    write_manifest(pdir, &man)?;
    let _ = fs::set_permissions(pdir, fs::Permissions::from_mode(0o700));

    Ok(Profile {
        is_current: true,
        name: man.name,
        created_at: man.created_at,
        uid: man.uid,
        fingerprint: man.fingerprint,
        session_len: man.session_len,
        account_label: man.account_label,
        size_kb: dir_size(pdir) / 1024,
        path: pdir.to_path_buf(),
    })
}

/// 保存当前登录态为档案（带校验：名称非空、不重名、不重复账号、当前必须有登录票据）
pub fn save_profile(name: &str) -> Result<Profile, String> {
    save_profile_inner(name, false)
}

/// `allow_dup` 仅给自检用：跳过「同一账号只存一份」的限制
fn save_profile_inner(name: &str, allow_dup: bool) -> Result<Profile, String> {
    let safe = sanitize(name);
    if safe.is_empty() {
        return Err("名称不能为空".into());
    }
    // 名称重复校验
    let profiles = profiles_dir();
    let pdir = profiles.join(&safe);
    if pdir.exists() {
        return Err(format!("「{}」已存在，换个名字或先删掉旧的", safe));
    }
    // 有效性校验：当前必须有登录票据
    let cur = current_session_len().unwrap_or(0);
    if cur == 0 {
        return Err(
            "当前没有登录票据（WorkBuddy 处于未登录状态），请先登录后再保存".into(),
        );
    }
    // 账号去重：同一个 uid 只保留一份档案（名字可以不同，账号是同一个就没意义存两份）
    if !allow_dup {
        if let Some(cur_uid) = detect_uid() {
            if let Some(dup) = list_profiles()
                .into_iter()
                .find(|p| p.uid.as_deref() == Some(cur_uid.as_str()))
            {
                return Err(format!(
                    "当前这个账号已经存过了（「{}」）。同一个账号不需要保存两份，要更新就删掉旧的再存",
                    dup.name
                ));
            }
        }
    }
    fs::create_dir_all(&profiles).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&profiles, fs::Permissions::from_mode(0o700));
    save_current_to(&pdir, &safe)
}

pub fn delete_profile(name: &str) -> Result<(), String> {
    let pdir = profiles_dir().join(sanitize(name));
    if !pdir.exists() {
        return Err("档案不存在".into());
    }
    remove_all(&pdir).map_err(|e| e.to_string())
}

/// 改名（目录随名字变）
pub fn rename_profile(old: &str, new: &str) -> Result<Profile, String> {
    let new_safe = sanitize(new);
    if new_safe.is_empty() {
        return Err("名称不能为空".into());
    }
    let old_dir = profiles_dir().join(sanitize(old));
    if !old_dir.exists() {
        return Err("档案不存在".into());
    }
    let new_dir = profiles_dir().join(&new_safe);
    if new_dir != old_dir && new_dir.exists() {
        return Err(format!("「{}」已存在，换个名字", new_safe));
    }
    if new_dir != old_dir {
        fs::rename(&old_dir, &new_dir).map_err(|e| format!("改名失败：{}", e))?;
    }
    let mut man = read_manifest(&new_dir);
    man.name = new_safe.clone();
    write_manifest(&new_dir, &man)?;
    Ok(list_profiles()
        .into_iter()
        .find(|p| p.name == new_safe)
        .unwrap_or(Profile {
            name: new_safe,
            created_at: man.created_at,
            uid: man.uid,
            fingerprint: man.fingerprint,
            session_len: man.session_len,
            account_label: man.account_label,
            path: new_dir.clone(),
            size_kb: dir_size(&new_dir) / 1024,
            is_current: false,
        }))
}

/// 档案里记录的账号 uid：先看 manifest，老档案没有就回落到档案内的账号快照
pub fn snapshot_uid(pdir: &Path) -> Option<String> {
    if let Some(u) = read_manifest(pdir).uid {
        if !u.is_empty() {
            return Some(u);
        }
    }
    let f = pdir
        .join("files")
        .join("storage/skeleton/account-snapshot.json");
    let text = fs::read_to_string(&f).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("primary")?
        .get("uid")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 这个档案是不是「当前已经登录的账号」。
/// 判据：uid 相同；uid 缺失时退化为票据指纹相同。
pub fn is_current_profile(name: &str) -> bool {
    let pdir = profiles_dir().join(sanitize(name));
    if let (Some(a), Some(b)) = (snapshot_uid(&pdir), detect_uid()) {
        if a == b {
            return true;
        }
    }
    match (snapshot_fingerprint(&pdir), current_fingerprint()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// 切换：先校验，再自动备份当前状态，最后整体还原目标档案
pub fn switch_to(name: &str) -> Result<String, String> {
    let pdir = profiles_dir().join(sanitize(name));
    let files_dir = pdir.join("files");
    if !files_dir.exists() {
        return Err(format!("档案不存在或已损坏：{}", name));
    }
    // 目标就是当前登录的这个账号 → 什么都不用做（还原一遍反而可能把新票据覆盖成旧的）
    if is_current_profile(name) {
        return Ok(format!("「{}」就是当前登录的账号，无需切换", name));
    }
    // 有效性校验：目标存档必须有票据
    if snapshot_session_len(&pdir).unwrap_or(0) == 0 {
        return Err(format!(
            "「{}」没有登录票据，切换过去也登不进去。请在登录该账号后重新保存一次",
            name
        ));
    }
    if wb_running() {
        return Err("WorkBuddy 正在运行，请先 Cmd+Q 退出后再切换".into());
    }
    // 清场：常驻 daemon（sandbox-center / node）持有凭证文件句柄，
    // 不先杀掉的话可能把内存里的旧凭证写回磁盘，把刚还原的档案覆盖掉
    kill_all_wb_procs();

    // 1. 自动备份当前状态
    let bak_name = backup_current()?;

    // 2. 还原目标档案
    restore_from(&pdir)?;

    Ok(format!("已切换到「{}」，切换前状态已备份为 {}", name, bak_name))
}

/// 备份当前状态到 backups/auto-<时间戳>，返回备份名
fn backup_current() -> Result<String, String> {
    let backups = backups_dir();
    fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&backups, fs::Permissions::from_mode(0o700));
    let stamp = now_string().replace(['-', ' ', ':'], "");
    let bak_name = format!("auto-{}", stamp);
    let bak_dir = backups.join(&bak_name);
    if let Err(e) = save_current_to(&bak_dir, &bak_name) {
        return Err(format!("备份当前状态失败：{}", e));
    }
    Ok(bak_name)
}

/// 把 daemon 的活凭证（*.info，不含带时间戳的备份 / .logged-out 标记 / observation）
/// 存进档案的 files-ext/auth/
fn save_ext_auth(pdir: &Path) {
    let ext = ext_auth_dir();
    let Ok(entries) = fs::read_dir(&ext) else {
        return;
    };
    let dst = pdir.join("files-ext").join("auth");
    for e in entries.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        // 活凭证文件名形如 workbuddy-desktop.info；
        // 备份形如 workbuddy-desktop.info.2026-10-13T…info，特征是多一段后缀
        if !is_live_cred_name(&n) {
            continue;
        }
        // 带 .logged-out 标记的凭证会被 App 忽略，存了也还原不出来
        if ext.join(format!("{}.logged-out", n)).exists() {
            continue;
        }
        let _ = fs::create_dir_all(&dst);
        let _ = copy_all(&e.path(), &dst.join(&n));
    }
}

/// 把档案里的 daemon 凭证还原回去，并删掉 .logged-out 登出标记
/// （标记存在时 App 启动会忽略凭证，直接显示未登录）
fn restore_ext_auth(pdir: &Path) {
    let ext = ext_auth_dir();
    let src = pdir.join("files-ext").join("auth");
    if src.exists() {
        let _ = fs::create_dir_all(&ext);
        if let Ok(entries) = fs::read_dir(&src) {
            for e in entries.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let _ = copy_all(&e.path(), &ext.join(&n));
                let _ = fs::set_permissions(ext.join(&n), fs::Permissions::from_mode(0o600));
            }
        }
    }
    clear_logout_markers();
}

/// 删除 daemon 凭证目录里的登出标记与观察文件
fn clear_logout_markers() {
    let ext = ext_auth_dir();
    if let Ok(entries) = fs::read_dir(&ext) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".logged-out") || n.ends_with(".observation.json") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// 把一个档案目录里的 files/ 整体还原回 ~/.workbuddy
fn restore_from(pdir: &Path) -> Result<(), String> {
    let files_dir = pdir.join("files");
    if !files_dir.exists() {
        return Err("档案不存在或已损坏".into());
    }
    let wb = wb_dir();
    let entries: Vec<_> = fs::read_dir(&files_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    for rel in entries {
        let src = files_dir.join(&rel);
        let dst = wb.join(&rel);
        remove_all(&dst).map_err(|e| e.to_string())?;
        copy_all(&src, &dst).map_err(|e| e.to_string())?;
    }
    restore_ext_auth(pdir);
    Ok(())
}

// ------------------------------------------------- 「后悔药」：最近一次有效登录态

/// 存放「最近一次有效登录态」的目录（滚动覆盖，只保留一份）
pub fn last_good_dir() -> PathBuf {
    store_dir().join("last-good")
}

/// 当前磁盘上登录票据的长度（0 / None 表示当前没登录）。
/// 兜底链：主 cookie 库（session cookie 明文长度）→ webview 分区 →
/// daemon 凭证（个人账号只有这一处，用凭证文件字节数作度量）
pub fn current_session_len() -> Option<usize> {
    let main = wb_dir().join("app/session/Cookies");
    if let Some(n) = session_len_of(&main) {
        if n > 0 {
            return Some(n);
        }
    }
    let parts = wb_dir().join("app/session/Partitions");
    if let Ok(entries) = fs::read_dir(&parts) {
        for e in entries.flatten() {
            let p = e.path().join("Cookies");
            if let Some(n) = session_len_of(&p) {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }
    current_daemon_cred().map(|c| c.bytes)
}

/// 当前登录票据的来源文件（用于排查「App 里已登录但工具检测不到」）
#[allow(dead_code)]
pub fn session_source() -> Option<PathBuf> {
    let main = wb_dir().join("app/session/Cookies");
    if session_len_of(&main).unwrap_or(0) > 0 {
        return Some(main);
    }
    let parts = wb_dir().join("app/session/Partitions");
    if let Ok(entries) = fs::read_dir(&parts) {
        for e in entries.flatten() {
            let p = e.path().join("Cookies");
            if session_len_of(&p).unwrap_or(0) > 0 {
                return Some(p);
            }
        }
    }
    live_cred_path()
}

/// 当前票据是从哪一级探测到的（界面显示用，方便一眼看出走的是 cookie 还是 daemon）
pub fn session_source_kind() -> &'static str {
    let main = wb_dir().join("app/session/Cookies");
    if session_len_of(&main).unwrap_or(0) > 0 {
        return "cookie";
    }
    let parts = wb_dir().join("app/session/Partitions");
    if let Ok(entries) = fs::read_dir(&parts) {
        for e in entries.flatten() {
            if session_len_of(&e.path().join("Cookies")).unwrap_or(0) > 0 {
                return "分区cookie";
            }
        }
    }
    if current_daemon_cred().is_some() {
        return "daemon";
    }
    "-"
}

/// 如果当前确实处于登录状态，就把登录态滚动存一份到 last-good。
/// 关键点：**当前没登录时绝不覆盖**，这样「刚点了退出账号」也能找回上一份有效票据。
pub fn maybe_update_last_good() {
    if current_session_len().unwrap_or(0) == 0 {
        return;
    }
    let dir = last_good_dir();
    let cur_fp = current_fingerprint();
    if dir.exists() {
        let old = read_manifest(&dir);
        // 票据没变就别重复拷贝
        if cur_fp.is_some() && old.fingerprint == cur_fp {
            return;
        }
    }
    let _ = save_current_to(&dir, "last-good");
}

/// 最近一次有效登录态的摘要信息（没有则 None）
pub fn last_good_info() -> Option<Profile> {
    let dir = last_good_dir();
    if !dir.join("files").exists() {
        return None;
    }
    let man = read_manifest(&dir);
    if man.session_len.unwrap_or(0) == 0 {
        return None;
    }
    Some(Profile {
        is_current: man.fingerprint == current_fingerprint(),
        name: "最近一次有效登录态".into(),
        created_at: man.created_at,
        uid: man.uid,
        fingerprint: man.fingerprint,
        session_len: man.session_len,
        account_label: man.account_label.or_else(|| snapshot_account_label(&dir)),
        size_kb: dir_size(&dir) / 1024,
        path: dir,
    })
}

/// 还原「最近一次有效登录态」
pub fn restore_last_good() -> Result<String, String> {
    if wb_running() {
        return Err("WorkBuddy 正在运行，请先 Cmd+Q 退出后再还原".into());
    }
    let dir = last_good_dir();
    if !dir.join("files").exists() {
        return Err("还没有记录过有效登录态".into());
    }
    let _ = backup_current();
    restore_from(&dir)?;
    Ok("已还原最近一次有效登录态".into())
}

/// 读某个快照里 session cookie 的明文长度，用来判断这份存档到底抓没抓到登录态
/// None = 没有 session cookie
pub fn snapshot_session_len(pdir: &Path) -> Option<usize> {
    session_len_of(&pdir.join("files").join("app/session/Cookies"))
        .or_else(|| saved_daemon_cred(pdir).map(|c| c.bytes))
}

/// 读某个 Cookies 库里 session cookie 的长度，用来判断抓没抓到登录态
/// None = 没有 session cookie
pub fn session_len_of(cookies: &Path) -> Option<usize> {
    if !cookies.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &cookies,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT name, value, encrypted_value FROM cookies WHERE host_key LIKE '%workbuddy.cn%'",
        )
        .ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                cell_bytes(r.get::<_, rusqlite::types::Value>(1)?),
                cell_bytes(r.get::<_, rusqlite::types::Value>(2)?),
            ))
        })
        .ok()?;
    // 先看 name='session'，其次任何长得像票据的 cookie
    let mut fallback: Option<usize> = None;
    for row in rows.flatten() {
        let (name, plain, enc) = row;
        let v = if !plain.is_empty() { plain } else { enc };
        if v.is_empty() {
            continue;
        }
        if name == "session" {
            return Some(v.len());
        }
        if matches!(name.as_str(), "token" | "access_token" | "refresh_token")
            && fallback.is_none()
        {
            fallback = Some(v.len());
        }
    }
    fallback
}

/// 读存档里的账号快照，得到「张三（某某公司 旗舰版）」这样的可读标签
pub fn snapshot_account_label(pdir: &Path) -> Option<String> {
    let f = pdir.join("files").join("storage/skeleton/account-snapshot.json");
    let text = fs::read_to_string(&f).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let p = v.get("primary")?;
    let nick = normalize_nickname(p.get("nickname").and_then(|x| x.as_str()).unwrap_or("未知"));
    let ent = p.get("enterpriseName").and_then(|x| x.as_str()).unwrap_or("");
    let typ = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
    let label = if !ent.is_empty() {
        format!("{}（{}·{}）", nick, ent, typ)
    } else if typ == "personal" {
        format!("{}（个人版）", nick)
    } else {
        format!("{}（{}）", nick, typ)
    };
    Some(label)
}

pub fn launch_workbuddy() -> Result<(), String> {
    // open 在刚 kill 完进程时可能静默失败，重试几次并确认真的拉起了
    let mut last_err = String::new();
    for _ in 0..20 {
        match Command::new("/usr/bin/open").args(["-a", "WorkBuddy"]).output() {
            Ok(o) if o.status.success() => {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if wb_running() {
                    return Ok(());
                }
            }
            Ok(o) => last_err = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) => last_err = e.to_string(),
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(if last_err.is_empty() {
        "open 命令已执行但 WorkBuddy 未起来，请手动打开".into()
    } else {
        format!("启动 WorkBuddy 失败：{}", last_err)
    })
}

/// 「登录新账号」：仅本地退出 —— 删除本机登录凭证，不请求服务端注销。
/// 这样旧账号的 session 在服务端继续有效（已保存的档案不受影响），
/// 同时 WorkBuddy 下次打开会回到登录页，可以登录第二个账号。
pub fn local_logout() -> Result<String, String> {
    if wb_running() {
        return Err("WorkBuddy 正在运行，请先退出".into());
    }
    // 先清场：杀掉常驻 daemon，避免它把内存里的旧凭证写回磁盘
    kill_all_wb_procs();

    // 1. 当前若已登录，先滚动留存到 last-good（后悔药）
    maybe_update_last_good();

    // 2. 删除本机登录凭证（不动服务端）
    let wb = wb_dir();
    let mut removed = 0usize;
    for rel in AUTH_ITEMS {
        let p = wb.join(rel);
        if p.exists() && remove_all(&p).is_ok() {
            removed += 1;
        }
    }
    // wb-webview 分区 + 账号快照 + 用户态目录（与 App 内退出时的本地清理对齐）
    let parts = wb.join("app/session/Partitions");
    if let Ok(entries) = fs::read_dir(&parts) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("wb-webview-") {
                let _ = remove_all(&e.path());
            }
        }
    }
    for extra in ["storage/skeleton", "app/users"] {
        let p = wb.join(extra);
        if p.exists() {
            let _ = remove_all(&p);
        }
    }
    // daemon 凭证：活凭证 + 登出标记 + 观察文件（带时间戳的历史备份保留）
    let ext = ext_auth_dir();
    if let Ok(entries) = fs::read_dir(&ext) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let live = is_live_cred_name(&n);
            if live || n.ends_with(".logged-out") || n.ends_with(".observation.json") {
                let _ = fs::remove_file(e.path());
                removed += 1;
            }
        }
    }
    Ok(format!(
        "已清除本机登录信息（{} 项），旧账号在服务端保持有效；打开 WorkBuddy 即可登录新账号",
        removed
    ))
}

pub fn quit_workbuddy() -> Result<(), String> {
    Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"WorkBuddy\" to quit"])
        .status()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// App 运行中也能一致性导出 Cookies（VACUUM INTO）
    #[test]
    fn cookies_consistent_export() {
        if !wb_running() {
            println!("（WorkBuddy 未运行，跳过运行时导出测试）");
            return;
        }
        let dst = std::path::PathBuf::from("/tmp/wbswitch-cookies-test.db");
        let _ = std::fs::remove_file(&dst);
        copy_cookies(&wb_dir().join("app/session/Cookies"), &dst).unwrap();
        let size = std::fs::metadata(&dst).unwrap().len();
        assert!(size > 0, "导出的 Cookies 不应为空");
        let conn = rusqlite::Connection::open_with_flags(
            &dst,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM cookies", [], |r| r.get(0))
            .unwrap();
        println!("运行中导出成功：{} 字节，{} 条 cookie", size, n);
        assert!(n > 0);
        let _ = std::fs::remove_file(&dst);
    }

    /// 票据三级兜底：主库 → webview 分区 → daemon 凭证。
    /// 个人账号的票据只在 daemon 凭证里，没有这一级就会被误判成「未登录」。
    #[test]
    fn session_detection_three_levels() {
        let main = wb_dir().join("app/session/Cookies");
        let main_len = session_len_of(&main).unwrap_or(0);
        let daemon = current_daemon_cred();
        println!(
            "主库票据={} daemon 凭证={:?}",
            main_len,
            daemon.as_ref().map(|c| format!("uid={} type={} bytes={}", c.uid, c.typ, c.bytes))
        );
        // 只要 App 里是登录状态，就必须至少从某一级探测到票据
        let total = current_session_len().unwrap_or(0);
        assert!(
            total > 0,
            "当前处于登录状态但三级兜底都没探测到票据（daemon 凭证是否漏了？）"
        );
        assert!(current_fingerprint().is_some(), "探测到票据就必须有指纹");
        // 有票据时 uid 一定可解析
        assert!(detect_uid().is_some(), "登录状态下必须能识别 uid");
    }

    /// 保存 → 读回：档案的票据/指纹/uid 必须能自洽（个人账号只有 daemon 凭证）
    #[test]
    fn saved_profile_roundtrip() {
        if current_session_len().unwrap_or(0) == 0 {
            println!("（当前未登录，跳过）");
            return;
        }
        let name = "__roundtrip";
        let _ = delete_profile(name);
        let p = save_profile_inner(name, true).expect("登录状态下保存应成功");
        assert!(
            p.session_len.unwrap_or(0) > 0,
            "保存下来的档案必须带票据（个人账号应走 daemon 凭证兜底）"
        );
        assert!(p.uid.is_some(), "档案必须记录 uid");
        // 读回时指纹/票据长度必须与当前一致
        let back = list_profiles().into_iter().find(|x| x.name == name);
        let back = back.expect("保存后应能列出该档案");
        assert!(back.is_current, "刚保存的档案应被标记为当前账号");
        assert_eq!(back.session_len, p.session_len);
        assert_eq!(back.fingerprint, current_fingerprint());
        let _ = delete_profile(name);
    }

    /// 新规则（v9）：
    /// 1) 切到「当前已登录的账号」应该是无动作的成功提示；
    /// 2) 同一个 uid 不允许存第二份档案。
    #[test]
    fn current_account_noop_and_uid_dedupe() {
        if current_session_len().unwrap_or(0) == 0 {
            println!("（当前未登录，跳过）");
            return;
        }
        let existing = list_profiles()
            .into_iter()
            .find(|p| p.uid.is_some() && p.uid == detect_uid());
        let (name, temp) = match existing {
            Some(p) => (p.name, false),
            None => {
                let n = "__dedupe_base".to_string();
                save_profile_inner(&n, true).expect("准备基础档案失败");
                (n, true)
            }
        };

        assert!(is_current_profile(&name), "当前账号的档案必须被判为「当前」");
        let msg = switch_to(&name).expect("切到当前账号不应报错");
        println!("切到当前账号 -> {}", msg);
        assert!(msg.contains("无需切换"), "应当明确告知无需切换");

        let err = match save_profile("__dedupe_dup") {
            Ok(_) => panic!("同一账号不允许存第二份"),
            Err(e) => e,
        };
        println!("uid 去重提示 -> {}", err);
        assert!(
            err.contains(&name) || err.contains("已经存过"),
            "去重提示里应该点名已有档案「{}」，实际：{}",
            name,
            err
        );
        let _ = delete_profile("__dedupe_dup");
        if temp {
            let _ = delete_profile("__dedupe_base");
        }
    }

    /// 能从快照里读出 session 票据长度
    #[test]
    fn snapshot_has_session_len() {
        for p in list_profiles() {
            let len = snapshot_session_len(&p.path);
            println!("档案 {} -> session 票据长度 {:?}", p.name, len);
        }
    }

    /// 自检：现在（已登录状态下）存一份，看票据和账号标签抓不抓得到
    #[test]
    fn selfcheck_save_now() {
        let name = "__selfcheck";
        let _ = delete_profile(name); // 先清掉上次的，避免重名校验拦截
        match save_profile_inner(name, true) {
            Ok(p) => {
                let ck = p.path.join("files/app/session/Cookies");
                let size = fs::metadata(&ck).map(|m| m.len()).unwrap_or(0);
                let rows = if let Ok(c) = rusqlite::Connection::open_with_flags(
                    &ck,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
                ) {
                    c.query_row("SELECT count(*) FROM cookies", [], |r| r.get::<_, i64>(0))
                        .unwrap_or(-1)
                } else {
                    -2
                };
                let sidecars: Vec<String> = fs::read_dir(ck.parent().unwrap())
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                println!(
                    "自检档案：票据={:?} 标签={:?}\n  Cookies 存在={} 大小={} 行数={}\n  目录内容={:?}",
                    p.session_len,
                    p.account_label,
                    ck.exists(),
                    size,
                    rows,
                    sidecars
                );
            }
            Err(e) => println!("自检保存失败：{}", e),
        }
        let _ = delete_profile(name); // 自检档案不留残渣
    }

    /// 「后悔药」：当前登录时会自动留存一份有效登录态
    #[test]
    fn last_good_snapshot() {
        maybe_update_last_good();
        match last_good_info() {
            Some(g) => println!(
                "last-good：{} 票据={:?} 标签={:?}",
                g.created_at, g.session_len, g.account_label
            ),
            None => println!("last-good：无（当前可能没登录）"),
        }
        if current_session_len().unwrap_or(0) > 0 {
            let g = last_good_info().expect("登录状态下应留存一份有效登录态");
            assert!(g.session_len.unwrap_or(0) > 0, "留存的登录态必须带票据");
        }
    }

    /// 校验逻辑：建议名、重名拒绝、改名
    #[test]
    fn save_validations() {
        let s = suggested_name();
        println!("建议名：{}", s);
        assert!(!s.is_empty());
        // 重名：先存一份再存同名 → 必须被拒
        let name = "__validcheck";
        let _ = delete_profile(name);
        if current_session_len().unwrap_or(0) == 0 {
            println!("（当前未登录，跳过保存类校验）");
            return;
        }
        let r1 = save_profile_inner(name, true);
        assert!(r1.is_ok(), "第一次保存应成功");
        let r2 = save_profile_inner(name, true);
        assert!(r2.is_err(), "重名保存必须被拒");
        println!("重名校验提示：{}", r2.err().unwrap());
        // 改名
        let r3 = rename_profile(name, "__validcheck2");
        assert!(r3.is_ok(), "改名应成功");
        let r4 = rename_profile("__validcheck2", name);
        assert!(r4.is_ok(), "改名回来应成功");
        let _ = delete_profile(name);
    }

    /// 验证能正确读取命令行版 wbswitch 保存的档案（字段命名一致）
    #[test]
    fn reads_cli_created_profiles() {
        let ps = list_profiles();
        for p in &ps {
            println!(
                "profile: name={} uid={:?} createdAt={:?} fp={:?} current={}",
                p.name, p.uid, p.created_at, p.fingerprint, p.is_current
            );
        }
        if ps.is_empty() {
            println!("（当前没有档案，跳过兼容性断言）");
            return;
        }
        assert!(
            ps.iter().any(|p| !p.created_at.is_empty()),
            "createdAt 不应为空（字段命名需与命令行版一致）"
        );
    }
}



