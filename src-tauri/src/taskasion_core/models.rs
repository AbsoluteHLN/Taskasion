//! 数据模型与 todo.md / goals.md 行级序列化。
//!
//! 真相源行格式(人可读的 Markdown checkbox,行尾 HTML 注释为机器元数据):
//!
//! ```markdown
//! - [ ] 交季度报告 <!-- id:a1b2c3d4 due:2026-09-18 pri:p1 tags:work src:human created:2026-09-14T12:00:00 -->
//! ```
//!
//! 键值以空格分隔,空字段直接省略;备注 note 可含空格,写成引号包裹的
//! `note:"多词备注"`(内部 " 和 \ 需转义),其余 value 内不允许空格(tags 用逗号)。
//! 与原 Python 版 `taskasion_core.models` 行为逐行等价,保证真相源文件双向兼容。

use chrono::Local;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub source: String,
    pub created: String,
    pub done_at: Option<String>,
    pub note: Option<String>,
}

/// 8 位小写 hex,与 Python 版 uuid4().hex[:8] 同格式。
pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// 本地时间 ISO 秒级,如 2026-09-17T15:24:33(与 Python datetime.now().isoformat 一致)。
pub fn now_ts() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 行尾注释里允许的键(meta)。
const META_KEYS: [&str; 8] = ["id", "due", "pri", "tags", "src", "created", "done", "note"];

/// 解析后的键值对:同名键后写覆盖先写(与 Python dict 一致)。
type Meta = Vec<(String, String)>;

fn meta_get<'a>(meta: &'a Meta, key: &str) -> Option<&'a str> {
    meta.iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// 读取以 `note:"` 开头的引号值(调用方已定位到内容起点)。
/// 返回 (反转义后的内容, 消费到字符串末尾的字节数,是否遇到闭合引号)。
/// `\"` 等转义保留原字符(Python re.sub(r"\\(.)", r"\1") 语义)。
fn take_quoted(s: &str) -> (String, usize, bool) {
    let mut out = String::new();
    let mut used = 0usize;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            return (out, used + 1, true);
        }
        if c == '\\' {
            match chars.next() {
                Some(e) => {
                    out.push(e);
                    used += 1 + e.len_utf8();
                }
                None => return (out, used, false),
            }
        } else {
            out.push(c);
            used += c.len_utf8();
        }
    }
    (out, used, false)
}

/// 解析 meta 文本:先摘出所有引号包裹的 note(可含空格),剩余 token 按空白切分。
fn parse_meta(raw: &str) -> Meta {
    let mut meta: Meta = Vec::new();
    let mut rest = raw.to_string();
    loop {
        match rest.find("note:\"") {
            Some(p) => {
                let (val, used, closed) = take_quoted(&rest[p + 6..]);
                if !closed {
                    // 引号未闭合:与 Python 正则一样视为不匹配,整段走普通 token
                    break;
                }
                meta.push(("note".to_string(), val));
                // 摘出引号段后,前后两段要拼回去,前面的 token 才不会丢
                let mut merged = String::with_capacity(rest.len());
                merged.push_str(&rest[..p]);
                merged.push_str(&rest[p + 6 + used..]);
                rest = merged;
            }
            None => break,
        }
    }
    for token in rest.split_whitespace() {
        if let Some((key, value)) = token.split_once(':') {
            if META_KEYS.contains(&key) && !value.is_empty() {
                meta.push((key.to_string(), value.to_string()));
            }
        }
    }
    meta
}

/// 摘出正文里的行尾注释:与 Python `<!--(.*?)-->\s*$` 语义一致 ——
/// 从第一个 `<!--` 起、到其后最后一个 `-->` 止,且注释后只剩空白才算 meta。
/// 返回 (注释前标题, meta 对)。无匹配时标题原样返回、meta 为空。
fn split_meta(body: &str) -> (String, Meta) {
    let mut start = None;
    let mut from = 0usize;
    while let Some(i) = body[from..].find("<!--") {
        let s = from + i;
        if let Some(end_rel) = body[s + 4..].rfind("-->") {
            let e = s + 4 + end_rel;
            if body[e + 3..].trim().is_empty() {
                start = Some((s, e));
                break;
            }
        }
        from = s + 4;
        if from >= body.len() {
            break;
        }
    }
    match start {
        Some((s, e)) => {
            let title = body[..s].trim_end().to_string();
            (title, parse_meta(&body[s + 4..e]))
        }
        None => (body.to_string(), Vec::new()),
    }
}

/// 单行 → Task;非 checkbox 行返回 None(正文行之外的一切都会被忽略)。
pub fn parse_line(line: &str) -> Option<Task> {
    let line = line.trim_end();
    let body = line.trim_start().strip_prefix("- [")?;
    let (mark, body) = body.split_once(']')?;
    if !matches!(mark, " " | "x" | "X") {
        return None;
    }
    let body = body.strip_prefix(' ')?;
    let (title, meta) = split_meta(body);
    let title = title.trim();
    let tags: Vec<String> = meta_get(&meta, "tags")
        .unwrap_or("")
        .split(',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    Some(Task {
        id: meta_get(&meta, "id").unwrap_or("").to_string(),
        title: if title.is_empty() { "(未命名)".into() } else { title.to_string() },
        done: mark.eq_ignore_ascii_case("x"),
        due: meta_get(&meta, "due").filter(|v| !v.is_empty()).map(str::to_string),
        priority: meta_get(&meta, "pri").filter(|v| !v.is_empty()).map(str::to_string),
        tags,
        source: meta_get(&meta, "src").filter(|v| !v.is_empty()).unwrap_or("human").to_string(),
        created: meta_get(&meta, "created").unwrap_or("").to_string(),
        done_at: meta_get(&meta, "done").filter(|v| !v.is_empty()).map(str::to_string),
        note: meta_get(&meta, "note").map(str::to_string),
    })
}

fn escape_note(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Task → 真相源行(与 Python render_line 字段顺序一致)。
pub fn render_line(t: &Task) -> String {
    let mut parts = vec![format!("id:{}", t.id)];
    if let Some(due) = &t.due {
        parts.push(format!("due:{due}"));
    }
    if let Some(pri) = &t.priority {
        parts.push(format!("pri:{pri}"));
    }
    if !t.tags.is_empty() {
        parts.push(format!("tags:{}", t.tags.join(",")));
    }
    parts.push(format!("src:{}", t.source));
    if !t.created.is_empty() {
        parts.push(format!("created:{}", t.created));
    }
    if let Some(done_at) = &t.done_at {
        parts.push(format!("done:{done_at}"));
    }
    if let Some(note) = &t.note {
        parts.push(format!("note:\"{}\"", escape_note(note)));
    }
    let mark = if t.done { "x" } else { " " };
    format!("- [{mark}] {} <!-- {} -->", t.title, parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_render_roundtrip() {
        let line = r#"- [ ] 交季度报告 <!-- id:a1b2c3d4 due:2026-09-18 pri:p1 tags:work,urgent src:human created:2026-09-14T12:00:00 note:"附上 季度模板" -->"#;
        let t = parse_line(line).unwrap();
        assert_eq!(t.id, "a1b2c3d4");
        assert_eq!(t.title, "交季度报告");
        assert_eq!(t.due.as_deref(), Some("2026-09-18"));
        assert_eq!(t.priority.as_deref(), Some("p1"));
        assert_eq!(t.tags, vec!["work", "urgent"]);
        assert_eq!(t.note.as_deref(), Some("附上 季度模板"));
        assert_eq!(render_line(&t), line);
    }

    #[test]
    fn note_escape_roundtrip() {
        let t = Task {
            id: "abcd1234".into(),
            title: "引号任务".into(),
            done: false,
            due: None,
            priority: None,
            tags: vec![],
            source: "human".into(),
            created: "2026-09-17T10:00:00".into(),
            done_at: None,
            note: Some("他说:\"你好\" \\ 降调".into()),
        };
        let line = render_line(&t);
        let back = parse_line(&line).unwrap();
        assert_eq!(back.note.as_deref(), Some("他说:\"你好\" \\ 降调"));
    }

    #[test]
    fn non_checkbox_ignored() {
        assert!(parse_line("# 标题").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("普通文本行").is_none());
    }
}
