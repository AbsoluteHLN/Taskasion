//! 提醒调度(Reminder)—— 只由桌面壳进程运行。
//!
//! 边界刻意收得很紧:
//! - **只读**:每轮扫描通过 `TaskStore::list` 拿当前任务,从不写 `todo.md`;
//!   删除/完成提醒的唯一方式就是删任务或清 due / remind_time,不存在第二份状态。
//! - **无新语法**:触发时刻 = `due`(日期)+ `remind_time`(可选时刻),
//!   领域判断在 [`taskasion_core::models::reminder_at`],这里只做"到点没有"和去重。
//! - **单次**:命中后同一个任务在同一个分钟内只响一次;命中的那一分钟过后不再补响
//!   (错过就是错过,避免开机后一次性炸出一串陈年提醒)。
//! - **声音内嵌**:提示音在运行时合成一段约 1.15s 的柔和风铃 WAV 直接喂给
//!   `PlaySoundW(SND_MEMORY)`,exe 里不带任何音频资源文件,保持单文件绿色分发。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use serde_json::json;
use tauri::Emitter;

use crate::taskasion_core::models::{reminder_at, Task};
use crate::taskasion_core::rest::Core;

/// 扫描间隔。10–15s 足以让"14:30 的提醒"在 14:30:0x 出现,CPU 占用可忽略。
const SCAN_INTERVAL: Duration = Duration::from_secs(12);

/// 触发窗口(秒):`target <= now < target + 窗口` 才算命中。
const FIRE_WINDOW_SECS: i64 = 60;

/// 去重记录保留时长(秒)。过一天即可丢弃,防止长跑进程无限增长。
const DEDUPE_TTL_SECS: i64 = 86_400;

/// 一次提醒命中。
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub due: String,
    pub remind_time: String,
}

impl Hit {
    /// 去重键:同一任务 + 同一触发分钟。重启后同一分钟内可能再响一次,可接受。
    fn key(&self) -> String {
        format!("{}@{} {}", self.id, self.due, self.remind_time)
    }
}

/// 纯函数:在当前时刻 `now` 应该响的任务。抽出来便于单测(不碰线程、不碰声音)。
pub fn hits_from(tasks: &[Task], now: NaiveDateTime) -> Vec<Hit> {
    let mut hits = Vec::new();
    for t in tasks {
        if t.done {
            continue;
        }
        let Some(at) = reminder_at(t) else { continue };
        let delta = (now - at).num_seconds();
        if !(0..FIRE_WINDOW_SECS).contains(&delta) {
            continue;
        }
        hits.push(Hit {
            id: t.id.clone(),
            title: t.title.clone(),
            due: t.due.clone().unwrap_or_default(),
            remind_time: t.remind_time.clone().unwrap_or_default(),
        });
    }
    hits
}

/// 去重:清理过期记录后,挑出本轮真正要响的命中(已响过的同一分钟直接跳过)。
/// 抽成纯函数,单测才能在不碰线程、不碰声音的前提下验证"不会重复响"。
fn take_fresh(
    fired: &mut HashMap<String, NaiveDateTime>,
    hits: Vec<Hit>,
    now: NaiveDateTime,
) -> Vec<Hit> {
    // 清理过期的去重记录(未来时刻理论上不会出现,保留即可)
    fired.retain(|_, at| (now - *at).num_seconds() < DEDUPE_TTL_SECS);
    let mut fresh = Vec::new();
    for hit in hits {
        if fired.contains_key(&hit.key()) {
            continue;
        }
        fired.insert(hit.key(), now);
        fresh.push(hit);
    }
    fresh
}

/// 启动调度线程。`core` 与 REST 共用同一个实例,因此人在界面上改的提醒
/// 下一轮扫描(≤12s)就会生效,不需要重启。
pub fn spawn(core: Arc<Core>, app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut fired: HashMap<String, NaiveDateTime> = HashMap::new();
        loop {
            std::thread::sleep(SCAN_INTERVAL);
            let now = Local::now().naive_local();

            // "todo" 即未完成;已完成、已删除的任务自然不会再出现
            let tasks = core.store.list("todo", None);
            for hit in take_fresh(&mut fired, hits_from(&tasks, now), now) {
                play_chime();
                let _ = app.emit(
                    "reminder-fired",
                    json!({
                        "id": hit.id,
                        "title": hit.title,
                        "due": hit.due,
                        "remind_time": hit.remind_time,
                    }),
                );
            }
        }
    });
}

// ---------------------------------------------------------------- 提示音

const SAMPLE_RATE: u32 = 44_100;
const DURATION_SECS: f32 = 1.15;

/// 合成一段柔和的"玻璃风铃"提示音(44.1kHz / 16bit / 单声道 WAV)。
///
/// 四个非谐波整数倍的分音叠加,8ms 起振 + 指数衰减,峰值压到 0.30 ——
/// 听感是一次轻响,不是 Windows 默认提示音,也不是连续蜂鸣。
fn build_chime_wav() -> Vec<u8> {
    const PARTIALS: [(f32, f32); 4] =
        [(880.0, 1.0), (1320.0, 0.55), (1760.0, 0.30), (2637.0, 0.18)];
    let total = (SAMPLE_RATE as f32 * DURATION_SECS) as usize;
    let mut raw: Vec<f32> = Vec::with_capacity(total);
    let mut peak = 0.0f32;
    for i in 0..total {
        let t = i as f32 / SAMPLE_RATE as f32;
        let attack = (t / 0.008).min(1.0);
        let decay = (-t / 0.28).exp();
        let mut s = 0.0f32;
        for (freq, amp) in PARTIALS {
            s += amp * (2.0 * std::f32::consts::PI * freq * t).sin();
        }
        let s = s * attack * decay;
        peak = peak.max(s.abs());
        raw.push(s);
    }

    let gain = if peak > 0.0 { 0.30 / peak } else { 0.0 };
    let fade = (SAMPLE_RATE as f32 * 0.02) as usize;
    let mut pcm: Vec<u8> = Vec::with_capacity(total * 2);
    for (i, s) in raw.iter().enumerate() {
        let tail = total.saturating_sub(i + 1);
        let f = if tail < fade { tail as f32 / fade as f32 } else { 1.0 };
        let v = (s * gain * f * 32_767.0).clamp(-32_768.0, 32_767.0) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }

    let data_len = pcm.len() as u32;
    let mut wav: Vec<u8> = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt 块长度
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // 字节率
    wav.extend_from_slice(&2u16.to_le_bytes()); // 块对齐
    wav.extend_from_slice(&16u16.to_le_bytes()); // 位深
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

#[cfg(windows)]
fn play_chime() {
    use std::sync::OnceLock;

    // SND_ASYNC 要求内存在播放期间一直有效,因此缓存到进程级 static。
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    let data = WAV.get_or_init(build_chime_wav);

    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_MEMORY: u32 = 0x0004;

    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(psz_sound: *const u16, hmod: isize, fdw_sound: u32) -> i32;
    }
    unsafe {
        PlaySoundW(data.as_ptr() as *const u16, 0, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(not(windows))]
fn play_chime() {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn task(id: &str, due: Option<&str>, remind: Option<&str>, done: bool) -> Task {
        Task {
            id: id.to_string(),
            title: format!("任务 {id}"),
            done,
            due: due.map(str::to_string),
            remind_time: remind.map(str::to_string),
            priority: None,
            tags: Vec::new(),
            source: "human".to_string(),
            created: String::new(),
            done_at: None,
            note: None,
        }
    }

    fn at(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 24).unwrap().and_hms_opt(h, m, s).unwrap()
    }

    #[test]
    fn fires_within_window_only() {
        let t = task("a", Some("2026-09-24"), Some("14:30"), false);
        assert!(hits_from(std::slice::from_ref(&t), at(14, 30, 0)).len() == 1);
        assert!(hits_from(std::slice::from_ref(&t), at(14, 30, 59)).len() == 1);
        // 早一秒还没到点
        assert!(hits_from(std::slice::from_ref(&t), at(14, 29, 59)).is_empty());
        // 超过窗口不补响
        assert!(hits_from(std::slice::from_ref(&t), at(14, 31, 0)).is_empty());
        assert!(hits_from(std::slice::from_ref(&t), at(18, 0, 0)).is_empty());
    }

    #[test]
    fn needs_both_due_and_valid_time() {
        let tasks = [
            task("no-due", None, Some("14:30"), false),
            task("no-remind", Some("2026-09-24"), None, false),
            task("bad-time", Some("2026-09-24"), Some("25:00"), false),
            task("other-day", Some("2026-09-25"), Some("14:30"), false),
        ];
        assert!(hits_from(&tasks, at(14, 30, 5)).is_empty());
    }

    #[test]
    fn done_tasks_never_fire() {
        let t = task("d", Some("2026-09-24"), Some("14:30"), true);
        assert!(hits_from(&[t], at(14, 30, 5)).is_empty());
    }

    #[test]
    fn hit_carries_id_and_time_for_dedupe() {
        let t = task("abc", Some("2026-09-24"), Some("9:5"), false);
        let hits = hits_from(&[t], at(9, 5, 30));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "abc");
        assert_eq!(hits[0].remind_time, "9:5", "原样带出,不做二次加工");
        assert_eq!(hits[0].key(), "abc@2026-09-24 9:5");
    }

    #[test]
    fn no_duplicate_fire_within_the_same_minute() {
        let t = task("a", Some("2026-09-24"), Some("14:30"), false);
        let mut fired = HashMap::new();

        // 第一轮扫描:响一次
        let first = take_fresh(&mut fired, hits_from(std::slice::from_ref(&t), at(14, 30, 5)), at(14, 30, 5));
        assert_eq!(first.len(), 1);

        // 同一分钟内后续几轮扫描:列表里任务还在,但不能再响
        for s in [10u32, 20, 40, 59] {
            let again = take_fresh(&mut fired, hits_from(std::slice::from_ref(&t), at(14, 30, s)), at(14, 30, s));
            assert!(again.is_empty(), "{s}s 时不应重复响");
        }
    }

    #[test]
    fn dedupe_entry_expires_after_ttl_but_not_before() {
        let t = task("a", Some("2026-09-24"), Some("14:30"), false);
        let hit = hits_from(std::slice::from_ref(&t), at(14, 30, 5)).remove(0);

        // 刚响过 → 同键仍在 TTL 内 → 不响
        let mut recent = HashMap::new();
        recent.insert(hit.key(), at(14, 30, 5));
        assert!(take_fresh(&mut recent, vec![hit.clone()], at(14, 30, 10)).is_empty());

        // 两天前的记录应被清理,不会无限增长,也不会永久压制同一个键
        let mut old = HashMap::new();
        old.insert(hit.key(), at(14, 30, 5) - chrono::Duration::days(2));
        assert_eq!(take_fresh(&mut old, vec![hit.clone()], at(14, 30, 10)).len(), 1);
        assert_eq!(old.len(), 1, "过期记录被清掉,只留本轮写入的一条");
    }

    #[test]
    fn chime_is_a_valid_wav_image() {
        let wav = build_chime_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let expected = 44 + (SAMPLE_RATE as f32 * DURATION_SECS) as usize * 2;
        assert_eq!(wav.len(), expected);
        // data 块长度字段必须与实际一致,否则 PlaySoundW 只放半截
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data_len, wav.len() - 44);
    }

    // ---------------- 端到端(真文件 × 真 TaskStore,对应手工测试清单)----------------

    use crate::taskasion_core::store::TaskStore;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("taskasion-reminder-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 今天未来 / 今天已过 / 明天三种 due,以及"错过不补响"。
    #[test]
    fn today_future_today_past_and_tomorrow() {
        let dir = tmp_dir("lifecycle");
        let store = TaskStore::new(&dir, None);

        let later = store.add("今天稍后", Some("2026-09-24"), Some("23:59"), None, None, "human", None).unwrap();
        let earlier = store.add("今天早已过去", Some("2026-09-24"), Some("00:01"), None, None, "human", None).unwrap();
        let tomorrow = store.add("明天的会", Some("2026-09-25"), Some("09:00"), None, None, "human", None).unwrap();

        let tasks = store.list("todo", None);
        assert_eq!(tasks.len(), 3);
        assert!(
            tasks.iter().any(|t| t.id == tomorrow.id && t.remind_time.as_deref() == Some("09:00")),
            "明天的提醒被原样保存"
        );
        assert!(hits_from(&tasks, at(12, 0, 0)).is_empty(), "中午不该响任何一条");

        let early = hits_from(&tasks, at(0, 1, 30));
        assert_eq!(early.len(), 1);
        assert_eq!(early[0].id, earlier.id, "00:01 的提醒在窗口内");
        assert!(hits_from(&tasks, at(0, 2, 0)).is_empty(), "00:01 的窗口过后不补响");

        let hits = hits_from(&tasks, at(23, 59, 20));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, later.id, "只有今天的 23:59 到点");
        assert!(hits_from(&tasks, at(9, 0, 30)).is_empty(), "明天 09:00 的提醒今天不响");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 完成、重开、改时间、清提醒、删除 —— 每一条都必须在下一轮扫描里立刻生效。
    #[test]
    fn completing_deleting_or_moving_a_reminder_stops_the_chime() {
        let dir = tmp_dir("mutate");
        let store = TaskStore::new(&dir, None);
        let t = store.add("吃药", Some("2026-09-24"), Some("14:30"), None, None, "human", None).unwrap();
        let now = at(14, 30, 10);
        assert_eq!(hits_from(&store.list("todo", None), now).len(), 1);

        // 完成的任务不在 todo 列表里 → 不再响;重新打开 → 回到计划里
        store.set_done(&t.id, true, "human").unwrap();
        assert!(hits_from(&store.list("todo", None), now).is_empty());
        store.set_done(&t.id, false, "human").unwrap();
        assert_eq!(hits_from(&store.list("todo", None), now).len(), 1);

        // 改时间 → 旧时刻失效,新时刻生效
        let mut fields = serde_json::Map::new();
        fields.insert("remind_time".into(), json!("14:45"));
        store.update(&t.id, &fields, "human").unwrap();
        assert!(hits_from(&store.list("todo", None), now).is_empty(), "改到 14:45 后 14:30 不该响");
        assert_eq!(hits_from(&store.list("todo", None), at(14, 45, 30)).len(), 1);

        // 清掉提醒 → 任何时刻都不响
        let mut clear = serde_json::Map::new();
        clear.insert("remind_time".into(), serde_json::Value::Null);
        store.update(&t.id, &clear, "human").unwrap();
        assert!(hits_from(&store.list("todo", None), at(14, 45, 30)).is_empty());

        // 清掉 due 也一样(提醒需要 due + remind_time 两个都在)
        let mut again = serde_json::Map::new();
        again.insert("remind_time".into(), json!("14:50"));
        store.update(&t.id, &again, "human").unwrap();
        let mut clear_due = serde_json::Map::new();
        clear_due.insert("due".into(), serde_json::Value::Null);
        store.update(&t.id, &clear_due, "human").unwrap();
        assert!(hits_from(&store.list("todo", None), at(14, 50, 10)).is_empty());

        // 删除 → 不响
        let mut back = serde_json::Map::new();
        back.insert("due".into(), json!("2026-09-24"));
        back.insert("remind_time".into(), json!("14:55"));
        store.update(&t.id, &back, "human").unwrap();
        assert_eq!(hits_from(&store.list("todo", None), at(14, 55, 10)).len(), 1);
        store.delete(&t.id, "human").unwrap();
        assert!(hits_from(&store.list("todo", None), at(14, 55, 10)).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 重启(新 store 从同一份 todo.md 读回)后提醒仍在,且同一分钟只响一次。
    #[test]
    fn reminder_survives_restart_and_never_double_fires() {
        let dir = tmp_dir("restart");
        let store = TaskStore::new(&dir, None);
        let t = store.add("交周报", Some("2026-09-24"), Some("09:05"), None, None, "human", None).unwrap();
        drop(store); // 关掉进程里的一切,只剩 todo.md

        let reopened = TaskStore::new(&dir, None);
        let tasks = reopened.list("todo", None);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, t.id);
        assert_eq!(tasks[0].remind_time.as_deref(), Some("09:05"), "提醒编码在 truth source 里,重启后读回");
        assert_eq!(hits_from(&tasks, at(9, 5, 30)).len(), 1);

        // 12s 扫描间隔 × 6 轮,同一分钟内只应响一次
        let mut fired = HashMap::new();
        let mut rings = 0;
        for s in [5u32, 17, 29, 41, 53, 59] {
            rings += take_fresh(&mut fired, hits_from(&reopened.list("todo", None), at(9, 5, s)), at(9, 5, s)).len();
        }
        assert_eq!(rings, 1, "同一分钟只响一次");
        // 窗口外的下一分钟:不补响
        assert!(take_fresh(&mut fired, hits_from(&reopened.list("todo", None), at(9, 6, 30)), at(9, 6, 30)).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 人直接手改 truth source 写进保留标签 → 下一轮扫描照样到点提醒。
    #[test]
    fn hand_edited_truth_source_reminder_is_picked_up() {
        let dir = tmp_dir("handedit");
        let store = TaskStore::new(&dir, None);
        std::fs::write(
            dir.join("todo.md"),
            "# Taskasion\n\n- [ ] 手写提醒 <!-- id:aaaabbbb due:2026-09-24 tags:work,_remind:07:00 -->\n",
        )
        .unwrap();

        let tasks = store.list("todo", None); // list 内部检测外部编辑
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "aaaabbbb");
        assert_eq!(tasks[0].remind_time.as_deref(), Some("07:00"));
        assert_eq!(tasks[0].tags, vec!["work"], "保留标签不会漏进普通标签");
        assert_eq!(hits_from(&tasks, at(7, 0, 30)).len(), 1);
        assert!(hits_from(&tasks, at(7, 1, 1)).is_empty(), "过了窗口就不再响");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
