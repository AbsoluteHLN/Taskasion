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
//!
//! 提醒(Reminder)不新增 meta 键,而是占用一个**保留标签** `_remind:HH:MM` 落在
//! 既有的 `tags:` 里 —— 关键词是"不改变 Markdown 语法":
//! - 旧版 Python / 旧版 Rust 只会把它当成一个普通标签原样保留,不丢数据;
//! - 对本进程之外的一切(REST / MCP / 前端)它都被翻译成外部字段 `remind_time`,
//!   且永远不会出现在 tags 列表里([`REMIND_PREFIX`] 开头的标签一律内部消化)。

use chrono::{Local, NaiveDateTime};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    /// 外部字段:`HH:MM`(24 小时制)。内部编码为保留标签 `_remind:HH:MM`。
    pub remind_time: Option<String>,
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub source: String,
    pub created: String,
    pub done_at: Option<String>,
    pub note: Option<String>,
}

/// 提醒的内部编码前缀。`tags:` 里以它开头的标签属于 Core 内部约定,
/// 解析时摘出、渲染时写回,绝不外露给 REST / MCP / 前端。
pub const REMIND_PREFIX: &str = "_remind:";

/// 规范化 `HH:MM`(容忍 `9:5` 这类简写)→ `09:05`;越界或非数字返回 None。
pub fn valid_remind(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (h, m) = raw.split_once(':')?;
    if h.is_empty() || m.is_empty() || h.len() > 2 || m.len() > 2 {
        return None;
    }
    if !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{hour:02}:{minute:02}"))
}

/// 任务的提醒触发时刻(本地时间):due + remind_time 缺任一或格式非法 → None。
/// 领域语义放在 models,调度器与测试共用同一份判断,避免第二套实现。
pub fn reminder_at(t: &Task) -> Option<NaiveDateTime> {
    let due = t.due.as_deref()?;
    let remind = t.remind_time.as_deref()?;
    let normalized = valid_remind(remind)?;
    let (h, m) = normalized.split_once(':')?;
    let date = chrono::NaiveDate::parse_from_str(due.trim(), "%Y-%m-%d").ok()?;
    date.and_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
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
    let raw_tags: Vec<String> = meta_get(&meta, "tags")
        .unwrap_or("")
        .split(',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    // 保留标签就地消化:合法的写成 remind_time,非法的也一并丢弃(内部分区不漏给外部)
    let mut remind_time: Option<String> = None;
    let mut tags: Vec<String> = Vec::with_capacity(raw_tags.len());
    for tag in raw_tags {
        match tag.strip_prefix(REMIND_PREFIX) {
            Some(v) => {
                if remind_time.is_none() {
                    remind_time = valid_remind(v);
                }
            }
            None => tags.push(tag),
        }
    }
    Some(Task {
        id: meta_get(&meta, "id").unwrap_or("").to_string(),
        title: if title.is_empty() { "(未命名)".into() } else { title.to_string() },
        done: mark.eq_ignore_ascii_case("x"),
        due: meta_get(&meta, "due").filter(|v| !v.is_empty()).map(str::to_string),
        remind_time,
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
/// 提醒在 tags 末尾追加内部保留标签(用户自己的标签顺序不受影响)。
pub fn render_line(t: &Task) -> String {
    let mut parts = vec![format!("id:{}", t.id)];
    if let Some(due) = &t.due {
        parts.push(format!("due:{due}"));
    }
    if let Some(pri) = &t.priority {
        parts.push(format!("pri:{pri}"));
    }
    let mut tags = t.tags.clone();
    if let Some(remind) = t.remind_time.as_deref().and_then(valid_remind) {
        tags.push(format!("{REMIND_PREFIX}{remind}"));
    }
    if !tags.is_empty() {
        parts.push(format!("tags:{}", tags.join(",")));
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
            remind_time: None,
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

    /// 提醒占用保留标签:外部只看到 remind_time,tags 里绝不出现 `_remind:*`,
    /// 且写回真相源时不破坏用户自己的标签顺序。
    #[test]
    fn remind_is_reserved_tag() {
        let line = r#"- [ ] 开会 <!-- id:aaaabbbb due:2026-09-24 tags:work,_remind:14:30 src:human created:2026-09-24T09:00:00 -->"#;
        let t = parse_line(line).unwrap();
        assert_eq!(t.remind_time.as_deref(), Some("14:30"));
        assert_eq!(t.tags, vec!["work"]); // 保留标签不泄漏
        let rendered = render_line(&t);
        assert!(rendered.contains("tags:work,_remind:14:30"), "{rendered}");
        // 往返稳定
        let back = parse_line(&rendered).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn remind_only_tag_yields_no_tags() {
        // 只有提醒时 tags 整体消失,但提醒仍在
        let t = Task {
            id: "aaaabbbb".into(),
            title: "喝水".into(),
            done: false,
            due: Some("2026-09-24".into()),
            remind_time: Some("9:5".into()), // 非规范写法也应被规范化
            priority: None,
            tags: vec![],
            source: "human".into(),
            created: "2026-09-24T09:00:00".into(),
            done_at: None,
            note: None,
        };
        let line = render_line(&t);
        assert!(line.contains("tags:_remind:09:05"), "{line}");
        assert_eq!(parse_line(&line).unwrap().remind_time.as_deref(), Some("09:05"));
    }

    #[test]
    fn invalid_remind_is_dropped_not_leaked() {
        let line = r#"- [ ] 坏数据 <!-- id:aaaabbbb due:2026-09-24 tags:work,_remind:25:99 src:human created:2026-09-24T09:00:00 -->"#;
        let t = parse_line(line).unwrap();
        assert_eq!(t.remind_time, None);
        assert_eq!(t.tags, vec!["work"]);
        assert_eq!(valid_remind("24:00"), None);
        assert_eq!(valid_remind("07:60"), None);
        assert_eq!(valid_remind("0700"), None);
        assert_eq!(valid_remind("07:30").as_deref(), Some("07:30"));
        assert_eq!(valid_remind("7:3").as_deref(), Some("07:03"));
    }

    #[test]
    fn reminder_at_needs_due_and_valid_time() {
        let mut t = parse_line("- [ ] x <!-- id:aaaabbbb due:2026-09-24 src:human -->").unwrap();
        assert!(reminder_at(&t).is_none()); // 无提醒
        t.remind_time = Some("14:30".into());
        let at = reminder_at(&t).unwrap();
        assert_eq!(at.to_string(), "2026-09-24 14:30:00");
        t.due = None;
        assert!(reminder_at(&t).is_none()); // 无日期无法定位
        // due 不做格式校验(与 Python 版一致,date 由人写);定位不到就静默不提醒,绝不猜
        t.due = Some("9/24".into());
        assert!(reminder_at(&t).is_none());
        t.due = Some("20260924".into());
        assert!(reminder_at(&t).is_none());
        // chrono 与 Python strptime 一样容忍省略前导零,保持两版行为一致
        t.due = Some("2026-9-24".into());
        assert_eq!(reminder_at(&t).unwrap().to_string(), "2026-09-24 14:30:00");
    }
}
