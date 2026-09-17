//! 追加式审计日志:所有任务变更(含 Agent 与外部编辑)记入 audit.jsonl。
//! 每行 `{ts, actor, action, task_id, detail}`(与 Python 版字段与顺序一致)。

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::models::now_ts;

pub struct Audit {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Audit {
    pub fn new(path: &Path) -> Audit {
        Audit { path: path.to_path_buf(), lock: Mutex::new(()) }
    }

    /// 追加一条审计记录(同一时刻只允许一个写入者)。
    pub fn append(&self, actor: &str, action: &str, task_id: Option<&str>, detail: &str) {
        // 手工拼行以保持与 Python 版一致的字段顺序:ts, actor, action, task_id, detail
        let line = format!(
            "{{\"ts\":\"{}\",\"actor\":{},\"action\":{},\"task_id\":{},\"detail\":{}}}",
            now_ts(),
            json_str(actor),
            json_str(action),
            match task_id {
                Some(id) => json_str(id),
                None => "null".to_string(),
            },
            json_str(detail),
        );
        let _guard = self.lock.lock().unwrap();
        if let Ok(mut fh) = fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(fh, "{line}");
        }
    }

    pub fn tail(&self, n: usize) -> Vec<serde_json::Value> {
        let _guard = self.lock.lock().unwrap();
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let mut out: Vec<serde_json::Value> = Vec::new();
        for line in text.lines().rev() {
            if out.len() >= n {
                break;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                out.push(v);
            }
        }
        out.reverse();
        out
    }
}

/// JSON 字符串转义:仅转义引号、反斜杠与控制字符,保留 UTF-8 原文
/// (等价于 Python json.dumps(..., ensure_ascii=False))。
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_tail() {
        let dir = std::env::temp_dir().join(format!("taskasion-audit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let audit = Audit::new(&dir.join("audit.jsonl"));
        audit.append("agent:test", "add", Some("abc"), "中文 detail \"引号\"");
        audit.append("human", "update", None, "");
        let tail = audit.tail(10);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0]["actor"], "agent:test");
        assert_eq!(tail[0]["action"], "add");
        assert_eq!(tail[0]["task_id"], "abc");
        assert_eq!(tail[0]["detail"], "中文 detail \"引号\"");
        // 字段顺序与 Python 版一致:ts, actor, action, task_id, detail
        let raw = fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        let line = raw.lines().next().unwrap();
        assert!(line.starts_with("{\"ts\":\""));
        assert!(line.contains("\"actor\":\"agent:test\",\"action\":\"add\""));
        let _ = fs::remove_dir_all(&dir);
    }
}
