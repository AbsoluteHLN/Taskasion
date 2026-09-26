import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  addGoal,
  addTask,
  deleteGoal,
  deleteTask,
  fetchGoals,
  fetchTasks,
  setDone,
  setGoalDone,
  updateTask,
  nowLocalTime,
  todayLocal,
  tomorrowLocal,
  type Goal,
  type Task,
} from "./api";

function dueLabel(due: string): { text: string; overdue: boolean } {
  const today = todayLocal();
  if (due < today) return { text: `⚠ ${due.slice(5)}`, overdue: true };
  if (due === today) return { text: "今天", overdue: false };
  return { text: due.slice(5), overdue: false };
}

const pct = (p: { total: number; done: number }) =>
  `${p.total ? (p.done / p.total) * 100 : 0}%`;

// HLN 条目入场动画的级联序号
const motionItem = (idx: number) => ({ "--hln-ui-motion-index": idx }) as CSSProperties;

// 长标题自动滚动:量一下内层是否溢出,溢出则把溢出像素交给 CSS 来回巡航。
// onClick 由行内提供:点标题 = 原地改标题(不弹模态)。
function MarqueeTitle({ text, onClick }: { text: string; onClick?: (e: MouseEvent) => void }) {
  const boxRef = useRef<HTMLSpanElement>(null);
  const innerRef = useRef<HTMLSpanElement>(null);
  const [dist, setDist] = useState(0);

  useLayoutEffect(() => {
    const box = boxRef.current;
    const inner = innerRef.current;
    if (!box || !inner) return;
    const over = Math.ceil(inner.scrollWidth - box.clientWidth);
    setDist(over > 2 ? over : 0);
  }, [text]);

  const scrollStyle =
    dist > 0
      ? ({
          "--scroll-dist": `${dist}px`,
          // 约 13px/s 的阅读速度,单程 4–14s:太快来不及认字,太慢等不到尾巴
          "--scroll-dur": `${Math.min(14, Math.max(4, dist / 13))}s`,
        } as CSSProperties)
      : undefined;

  return (
    <span
      ref={boxRef}
      className="title"
      data-scroll={dist > 0 ? "" : undefined}
      title="点击修改标题"
      onClick={onClick}
    >
      <span ref={innerRef} className="title-inner" style={scrollStyle}>
        {text}
      </span>
    </span>
  );
}

// 折叠态/展开态窗口逻辑尺寸;两种状态的上栏等高(52 = 6 边距 + 40 栏 + 6)
const SIZE_EXPANDED: [number, number] = [320, 440];
const SIZE_COLLAPSED: [number, number] = [320, 52];

function applyWindowSize([w, h]: [number, number]) {
  try {
    void getCurrentWindow()
      .setSize(new LogicalSize(w, h))
      .catch(() => {});
  } catch {
    // 纯浏览器环境(无 Tauri)忽略
  }
}

// 滚轮调时刻:HH:MM ± minutes(上加下减),环绕 24 小时
function nudgeTime(value: string, minutes: number): string {
  const [h, m] = value.split(":").map((x) => parseInt(x, 10));
  if (Number.isNaN(h) || Number.isNaN(m)) return value;
  const total = (((h * 60 + m + minutes) % 1440) + 1440) % 1440;
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

// "22:06" / "2206" / "9:5" → "22:06";空串 → ""(删除提醒);无法解析 → null(按取消处理)
function normalizeTime(raw: string): string | null {
  const v = raw.trim().replace("：", ":");
  if (!v) return "";
  let h: number;
  let m: number;
  if (v.includes(":")) {
    const [hs, ms] = v.split(":");
    if (!/^\d{1,2}$/.test(hs) || !/^\d{1,2}$/.test(ms)) return null;
    h = Number(hs);
    m = Number(ms);
  } else if (/^\d{4}$/.test(v)) {
    h = Number(v.slice(0, 2));
    m = Number(v.slice(2));
  } else {
    return null;
  }
  if (h > 23 || m > 59) return null;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

// 时刻滚轮输入框:纯文本(type=time 的原生弹层在无边框置顶小窗里不可靠,图标还挤占宽度)。
// 滚轮上下 ±5 分钟;回车/失焦提交(自动规范化),Esc 取消恢复原值。
function WheelTime(props: {
  className: string;
  title: string;
  initial: string;
  onCommit: (v: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const latest = useRef(props);
  latest.current = props;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const cur = el.value || latest.current.initial || nowLocalTime().slice(0, 5);
      el.value = nudgeTime(cur.slice(0, 5), e.deltaY < 0 ? 5 : -5);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);
  const commit = () => {
    const v = normalizeTime(ref.current?.value ?? "");
    if (v === null) latest.current.onCancel();
    else latest.current.onCommit(v);
  };
  return (
    <input
      ref={ref}
      type="text"
      inputMode="numeric"
      maxLength={5}
      placeholder="--:--"
      className={props.className}
      title={props.title}
      autoFocus
      defaultValue={props.initial}
      data-hln-ui-field
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        else if (e.key === "Escape") latest.current.onCancel();
      }}
      onBlur={commit}
    />
  );
}

const PRI_CYCLE: Record<string, string> = { "": "p1", p1: "p2", p2: "p3", p3: "" };

// 分组折叠偏好:未完成/已完成(含目标页的已完成)各自记住展开状态,重启后保持
const boolPref = (key: string, fallback: boolean) => {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === "1";
  } catch {
    return fallback;
  }
};
const flipPref =
  (key: string, set: (fn: (v: boolean) => boolean) => void) => () =>
    set((v) => {
      const n = !v;
      try {
        localStorage.setItem(key, n ? "1" : "0");
      } catch {
        // 隐私模式等存储不可用时静默降级为会话内状态
      }
      return n;
    });

export default function App() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [goals, setGoals] = useState<Goal[]>([]);
  const [view, setView] = useState<"tasks" | "goals">("tasks");
  const [title, setTitle] = useState("");
  const [due, setDue] = useState(todayLocal());
  const [pri, setPri] = useState("");
  const [note, setNote] = useState("");
  // 提醒:与日期同一行。◷ 只是入口,时刻输入按需展开(顶替备注框,不换行)
  const [remind, setRemind] = useState("");
  const [showRemind, setShowRemind] = useState(false);
  const remindBeforeEdit = useRef(""); // Esc 恢复用:打开时刻框那一刻的值
  const [goalTitle, setGoalTitle] = useState("");
  const [filter, setFilter] = useState<"today" | "all">("today");
  // 未完成/今日完成/归档 三段折叠:默认未完成与今日完成展开、归档收起;各自状态持久化
  const [showPending, setShowPending] = useState(() => boolPref("taskasion.tasksPending.open", true));
  const [showDoneToday, setShowDoneToday] = useState(() => boolPref("taskasion.tasksTodayDone.open", true));
  const [showArchive, setShowArchive] = useState(() => boolPref("taskasion.tasksArchive.open", false));
  const [showDone, setShowDone] = useState(() => boolPref("taskasion.goalsDone.open", false));
  const [collapsed, setCollapsed] = useState(false);
  const [offline, setOffline] = useState(false);
  const [openGoalId, setOpenGoalId] = useState<string | null>(null);
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  const [editingTitleId, setEditingTitleId] = useState<string | null>(null);
  const [editingRemindId, setEditingRemindId] = useState<string | null>(null);
  // 刚响过提醒的任务 id:短暂高亮,证明"是哪条在提醒"
  const [firedId, setFiredId] = useState<string | null>(null);
  // 更新控件状态:available/downloading → 常驻按钮;latest/error → 4s 后自动消失
  const [upd, setUpd] = useState<{ state: string; version: string | null; message: string | null } | null>(null);
  const updTimer = useRef<number | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ state: string; version: string | null; message: string | null }>("update-status", (e) => {
      const s = e.payload;
      setUpd(s);
      if (updTimer.current) window.clearTimeout(updTimer.current);
      if (s.state === "latest" || s.state === "error") {
        updTimer.current = window.setTimeout(() => setUpd(null), 4000);
      }
    })
      .then((fn) => (unlisten = fn))
      .catch(() => {});
    return () => {
      unlisten?.();
      if (updTimer.current) window.clearTimeout(updTimer.current);
    };
  }, []);

  // 提醒响铃:CSS 只负责"亮一下",不抢焦点、不弹窗、不改数据
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let timer: number | null = null;
    listen<{ id: string }>("reminder-fired", (e) => {
      const id = e.payload?.id;
      if (!id) return;
      setFiredId(id);
      if (timer) window.clearTimeout(timer);
      timer = window.setTimeout(() => setFiredId(null), 5000);
    })
      .then((fn) => (unlisten = fn))
      .catch(() => {});
    return () => {
      unlisten?.();
      if (timer) window.clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try {
        const [t, g] = await Promise.all([fetchTasks(), fetchGoals()]);
        if (alive) {
          setTasks(t);
          setGoals(g);
          setOffline(false);
        }
      } catch {
        if (alive) setOffline(true);
      }
    };
    poll();
    const timer = setInterval(poll, 1500);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  const refresh = async () => {
    try {
      const [t, g] = await Promise.all([fetchTasks(), fetchGoals()]);
      setTasks(t);
      setGoals(g);
      setOffline(false);
    } catch {
      setOffline(true);
    }
  };

  const openGoal = openGoalId ? (goals.find((g) => g.id === openGoalId) ?? null) : null;
  const pendingCount = tasks.filter((t) => !t.done).length;
  const activeGoals = goals.filter((g) => !g.done);
  const doneGoals = goals.filter((g) => g.done);

  const shown = useMemo(() => {
    if (openGoal) return tasks.filter((t) => !t.done && t.tags.includes(`goal:${openGoal.id}`));
    const today = todayLocal();
    const live = tasks.filter((t) => !t.done);
    return filter === "today" ? live.filter((t) => !t.due || t.due <= today) : live;
  }, [tasks, filter, openGoal]);

  const shownDone = useMemo(() => {
    if (openGoal) return tasks.filter((t) => t.done && t.tags.includes(`goal:${openGoal.id}`));
    return tasks.filter((t) => t.done);
  }, [tasks, openGoal]);

  // 今日完成 vs 归档:按 done_at 是否为今天分家;归档只在「全部」和目标详情出现,
  // 「今天」视图里过去完成的没有存在意义
  const shownDoneToday = useMemo(
    () => shownDone.filter((t) => (t.done_at ?? "").slice(0, 10) === todayLocal()),
    [shownDone],
  );
  const shownArchive = useMemo(() => shownDone.filter((t) => (t.done_at ?? "").slice(0, 10) !== todayLocal()), [shownDone]);
  const archiveVisible = openGoal ? true : filter === "all";

  const submitTask = async (e: FormEvent) => {
    e.preventDefault();
    const t = title.trim();
    if (!t) return;
    const remindValue = remind || null;
    // 只给了时刻没给日期 → 绑今天;若今天这个点已经过去,绑明天
    // (绝不落一个"设定即已过期"的提醒,那样用户永远等不到它响)
    const dueValue = due || (remindValue ? (remindValue > nowLocalTime() ? todayLocal() : tomorrowLocal()) : null);
    await addTask({
      title: t,
      due: dueValue,
      remind_time: remindValue,
      priority: pri || null,
      note: note.trim() || null,
      tags: openGoal ? [`goal:${openGoal.id}`] : undefined,
    });
    setTitle("");
    setPri("");
    setNote("");
    setRemind("");
    setShowRemind(false);
    setDue(todayLocal());
    await refresh();
  };

  // 刚被 blur 关闭的行内编辑框:同一击 click 会落在行上,短暂忽略防止关了又开(像卡死)
  const noteClosed = useRef<{ id: string; t: number } | null>(null);
  const titleClosed = useRef<{ id: string; t: number } | null>(null);
  // Esc 取消标记:输入框被卸载时若仍触发一次 blur,这里保证它不会顺手把内容存下去
  const cancelEdit = useRef(false);

  // 备注:回车/失焦保存,Esc 取消;清空 == 删除备注(写成 null,而不是空串)
  const saveNote = async (t: Task, raw: string) => {
    noteClosed.current = { id: t.id, t: Date.now() };
    setEditingNoteId(null);
    if (cancelEdit.current) {
      cancelEdit.current = false;
      return;
    }
    const v = raw.trim();
    if (v !== (t.note ?? "")) {
      await updateTask(t.id, { note: v || null });
      await refresh();
    }
  };

  // 标题:同样原地编辑;空白标题视为取消(任务必须有标题)
  const saveTitle = async (t: Task, raw: string) => {
    titleClosed.current = { id: t.id, t: Date.now() };
    setEditingTitleId(null);
    if (cancelEdit.current) {
      cancelEdit.current = false;
      return;
    }
    const v = raw.trim();
    if (v && v !== t.title) {
      await updateTask(t.id, { title: v });
      await refresh();
    }
  };

  const startTitleEdit = (t: Task) => (e: MouseEvent) => {
    e.stopPropagation();
    setEditingNoteId(null);
    setEditingTitleId(t.id);
  };

  // 提醒:清空时刻即删除提醒;补时刻时若任务还没有日期,按"今天/明天"补上
  const saveRemind = async (t: Task, raw: string) => {
    setEditingRemindId(null);
    if (cancelEdit.current) {
      cancelEdit.current = false;
      return;
    }
    const v = raw.trim();
    if (v === (t.remind_time ?? "")) return;
    const patch: { remind_time: string | null; due?: string } = { remind_time: v || null };
    if (v && !t.due) patch.due = v > nowLocalTime() ? todayLocal() : tomorrowLocal();
    await updateTask(t.id, patch);
    await refresh();
  };

  const toggleCollapse = () => {
    setCollapsed((c) => {
      applyWindowSize(c ? SIZE_EXPANDED : SIZE_COLLAPSED);
      return !c;
    });
  };

  // ---- 智能头部:单击 = 收起;按住拖过阈值 = 移窗(替代 data-tauri-drag-region,
  //      这样两种手势不互抢)。按钮上的按下不参与。 ----
  const headPress = useRef<{ x: number; y: number; t: number; moved: boolean } | null>(null);

  const headPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return;
    headPress.current = { x: e.clientX, y: e.clientY, t: Date.now(), moved: false };
  };

  const headPointerMove = (e: ReactPointerEvent) => {
    const p = headPress.current;
    if (!p || p.moved) return;
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 6) {
      p.moved = true;
      try {
        void getCurrentWindow().startDragging();
      } catch {
        // 纯浏览器环境忽略
      }
    }
  };

  const headPointerUp = () => {
    const p = headPress.current;
    headPress.current = null;
    if (p && !p.moved && Date.now() - p.t < 600) toggleCollapse();
  };

  // 迷你条:短按展开;长按(350ms)或按压拖动 → 系统级拖移窗口
  const miniPress = useRef<{ x: number; y: number; timer: number } | null>(null);
  const miniDrag = useRef(false);

  const miniPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return; // ✕ 等按钮不参与长按拖拽
    const x = e.clientX;
    const y = e.clientY;
    miniDrag.current = false;
    const timer = window.setTimeout(() => {
      miniPress.current = null; // 已进入系统拖移,清除按压态,防 hover 误触发
      miniDrag.current = true;
      try {
        void getCurrentWindow().startDragging();
      } catch {
        // 纯浏览器环境忽略
      }
    }, 350);
    miniPress.current = { x, y, timer };
  };

  const miniPointerMove = (e: ReactPointerEvent) => {
    const p = miniPress.current;
    if (!p) return;
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 6) {
      window.clearTimeout(p.timer);
      miniPress.current = null;
      miniDrag.current = true;
      try {
        void getCurrentWindow().startDragging();
      } catch {
        // 纯浏览器环境忽略
      }
    }
  };

  const miniPointerUp = () => {
    if (miniPress.current) window.clearTimeout(miniPress.current.timer);
    miniPress.current = null;
  };

  const miniClick = () => {
    if (miniDrag.current) {
      miniDrag.current = false; // 拖拽结束后的 click 不当点击
      return;
    }
    toggleCollapse();
  };

  const submitGoal = async (e: FormEvent) => {
    e.preventDefault();
    const g = goalTitle.trim();
    if (!g) return;
    await addGoal(g);
    setGoalTitle("");
    await refresh();
  };

  const toggleTask = async (t: Task) => {
    await setDone(t.id, !t.done);
    await refresh();
  };

  const removeTask = async (t: Task) => {
    await deleteTask(t.id);
    await refresh();
  };

  // 优先级循环:无 → P1 → P2 → P3 → 无
  const cyclePriority = async (t: Task) => {
    const next = PRI_CYCLE[t.priority ?? ""] ?? "";
    await updateTask(t.id, { priority: next || null });
    await refresh();
  };

  const taskRow = (t: Task, idx: number) => {
    const d = t.done ? null : t.due ? dueLabel(t.due) : null;
    const editing = editingNoteId === t.id;
    const editingTitle = editingTitleId === t.id;
    const editingRemind = editingRemindId === t.id;
    return (
      <li
        key={t.id}
        className={`task-row${t.done ? " done" : ""}`}
        data-hln-motion="item"
        data-hln-motion-state="enter"
        data-hln-motion-variant="data-stream"
        data-fired={firedId === t.id ? "" : undefined}
        style={motionItem(idx)}
        onClick={() => {
          // 标题/备注的编辑框刚因失焦关闭时,这一击不要再把编辑框打回来(像卡死)
          const nc = noteClosed.current;
          if (nc && nc.id === t.id && Date.now() - nc.t < 250) return;
          const tc = titleClosed.current;
          if (tc && tc.id === t.id && Date.now() - tc.t < 250) return;
          setEditingTitleId(null);
          setEditingNoteId(t.id);
        }}
      >
        <button
          className={`check${t.done ? " checked" : ""}`}
          title={t.done ? "回退" : "完成"}
          onClick={(e) => {
            e.stopPropagation();
            toggleTask(t);
          }}
        >
          ✓
        </button>
        {editingTitle ? (
          // 原地改标题:回车/失焦保存,Esc 取消(取消靠卸载输入框,onBlur 不会再补一次保存)
          <input
            className="title-edit"
            type="text"
            autoFocus
            defaultValue={t.title}
            data-hln-ui-field
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              else if (e.key === "Escape") {
                cancelEdit.current = true;
                titleClosed.current = { id: t.id, t: Date.now() };
                setEditingTitleId(null);
              }
            }}
            onBlur={(e) => saveTitle(t, e.currentTarget.value)}
          />
        ) : (
          <MarqueeTitle text={t.title} onClick={startTitleEdit(t)} />
        )}
        {d && <span className={`due${d.overdue ? " over" : ""}`}>{d.text}</span>}
        {!t.done &&
          (editingRemind ? (
            <WheelTime
              className="remind-edit"
              title="提醒时刻(框内滚轮上下 ±5 分钟;键入 22:06 / 2206 / 9:5 均可;清空并离开 = 删除提醒)"
              initial={t.remind_time ?? ""}
              onCommit={(v) => saveRemind(t, v)}
              onCancel={() => {
                cancelEdit.current = true;
                setEditingRemindId(null);
              }}
            />
          ) : (
            // 有提醒时是常显的 ◷ 14:30;没有时是幽灵 ◷,悬停行才亮,点开即设
            <button
              className="remind-btn"
              data-set={t.remind_time ? "" : undefined}
              title={t.remind_time ? `提醒 ${t.remind_time}(点击修改,清空即删除)` : "设置提醒时刻"}
              onClick={(e) => {
                e.stopPropagation();
                setEditingRemindId(t.id);
              }}
            >
              ◷{t.remind_time ? ` ${t.remind_time}` : ""}
            </button>
          ))}
        <button
          className={`pri${t.priority ? ` ${t.priority}` : " unset"}`}
          title="优先级:点击切换 无 → P1 → P2 → P3"
          onClick={(e) => {
            e.stopPropagation();
            cyclePriority(t);
          }}
        >
          {t.priority ? t.priority.toUpperCase() : "PRI"}
        </button>
        {t.tags.map((tag) =>
          tag.startsWith("goal:") ? (
            // 已在目标详情内:当前目标的徽章每行重复,不渲染,省高度防换行
            openGoal && tag === `goal:${openGoal.id}` ? null : (
              <span
                key={tag}
                className="tag goal-tag goal-dot"
                title={`目标: ${goals.find((g) => g.id === tag.slice(5))?.title ?? "目标"}`}
                onClick={(e: MouseEvent) => {
                  e.stopPropagation();
                  setView("goals");
                  setOpenGoalId(tag.slice(5));
                }}
              >
                ◎
              </span>
            )
          ) : (
            <span key={tag} className="tag">
              {tag}
            </span>
          ),
        )}
        {!t.done && t.source.startsWith("agent") && (
          <span className="agent" title={`来源: ${t.source}`}>
            AI
          </span>
        )}
        <button
          className="del"
          title="删除"
          onClick={(e) => {
            e.stopPropagation();
            removeTask(t);
          }}
        >
          ✕
        </button>
        {editing ? (
          <input
            className="note-edit"
            type="text"
            autoFocus
            defaultValue={t.note ?? ""}
            placeholder="备注:回车保存,Esc 取消"
            data-hln-ui-field
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              else if (e.key === "Escape") {
                cancelEdit.current = true;
                noteClosed.current = { id: t.id, t: Date.now() };
                setEditingNoteId(null);
              }
            }}
            onBlur={(e) => saveNote(t, e.currentTarget.value)}
          />
        ) : (
          // 备注默认收起,悬停条目时向下展开;没有备注时展开的是低存在感的"添加备注"提示。
          // 展开态与编辑态都在行内,所以"编辑中鼠标移开"不会把它收掉。
          <div className="note-wrap">
            <div className={t.note ? "note-line" : "note-line note-add"}>{t.note ?? "添加备注…"}</div>
          </div>
        )}
      </li>
    );
  };

  if (collapsed) {
    return (
      <div
        className="app collapsed"
        data-hln-ui-root
        data-hln-theme="arknights"
        data-hln-font="display"
        onPointerDown={miniPointerDown}
        onPointerMove={miniPointerMove}
        onPointerUp={miniPointerUp}
        onPointerCancel={miniPointerUp}
        onClick={miniClick}
      >
        <div className="mini" data-hln-ui-surface data-hln-ui-no-ornament title="点击展开 · 长按拖动">
          <span className="dot" />
          <span className="brand">TASKASION</span>
          <span className="count">{offline ? "OFFLINE" : `${pendingCount} 项`}</span>
          <button
            className="icon-btn"
            title="退出 Taskasion（含后台）"
            onClick={(e) => {
              e.stopPropagation(); // 不触发迷你条短按展开
              invoke("quit_app");
            }}
          >
            ✕
          </button>
        </div>
      </div>
    );
  }

  const meter = (p: { total: number; done: number }) => (
    <div className="tactical-meter secondary">
      <span style={{ width: pct(p) }} />
    </div>
  );

  return (
    <div
      className="app"
      data-hln-ui-root
      data-hln-ui-version="v2.3"
      data-hln-theme="arknights"
      data-hln-font="display"
      data-hln-bg-motion="tactical-grid"
    >
      <div className="panel" data-hln-ui-surface data-hln-ui-no-ornament data-hln-motion="panel" data-hln-motion-state="enter" data-hln-motion-variant="tactical-lock">
        <header
          className="head"
          data-hln-ui-bar
          title="单击收起 · 按住拖动"
          onPointerDown={headPointerDown}
          onPointerMove={headPointerMove}
          onPointerUp={headPointerUp}
          onPointerCancel={() => (headPress.current = null)}
        >
          <span className="brand">
            <span className="dot" />
            TASKASION
          </span>
          <span className="spacer" />
          {offline && <span className="offline">OFFLINE</span>}
          {upd && (
            <button
              className="icon-btn upd-btn"
              data-upd-state={upd.state}
              title={upd.message ?? ""}
              onClick={() => {
                if (upd.state === "available") void invoke("apply_update");
              }}
            >
              {upd.state === "available"
                ? "⤓"
                : upd.state === "downloading"
                  ? "···"
                  : upd.state === "latest"
                    ? "✓"
                    : upd.state === "error"
                      ? "!"
                      : "↻"}
            </button>
          )}
          <button className="icon-btn" title="退出 Taskasion（含后台）" onClick={() => invoke("quit_app")}>
            ✕
          </button>
        </header>

        {view === "goals" && openGoal ? (
          // 目标详情头部:压到约 40px 一行半。返回做成带边框的 26×26 方块 + CSS 箭头,
          // 比原来的 ‹ 字符更大更明确(字符在不同字体下粗细/位置不可控)。
          <div className="goal-head" data-tauri-drag-region>
            <button className="goal-back" title="返回目标列表" aria-label="返回目标列表" onClick={() => setOpenGoalId(null)}>
              <span className="chev" />
            </button>
            <div className="goal-head-info">
              <div className="goal-head-line">
                <span className="goal-head-title">{openGoal.title}</span>
                <span className="goal-head-nums">
                  {openGoal.progress.done}/{openGoal.progress.total}
                </span>
              </div>
              <div className="goal-head-meter">{meter(openGoal.progress)}</div>
            </div>
          </div>
        ) : (
          <nav className="viewbar" data-tauri-drag-region>
            <div className="segmented">
              <button data-hln-ui-control data-active={view === "tasks"} onClick={() => setView("tasks")}>
                任务 {pendingCount}
              </button>
              <button data-hln-ui-control data-active={view === "goals"} onClick={() => setView("goals")}>
                目标 {activeGoals.length}
              </button>
            </div>
            {view === "tasks" && !openGoal && (
              <div className="segmented">
                <button data-hln-ui-control data-active={filter === "today"} onClick={() => setFilter("today")}>
                  今天
                </button>
                <button data-hln-ui-control data-active={filter === "all"} onClick={() => setFilter("all")}>
                  全部
                </button>
              </div>
            )}
          </nav>
        )}

        {view === "goals" && !openGoal ? (
          <>
            <form className="adder" onSubmit={submitGoal}>
              <input
                type="text"
                value={goalTitle}
                onChange={(e) => setGoalTitle(e.target.value)}
                placeholder="持之以恒！"
                data-hln-ui-field
              />
              <button type="submit" className="add-btn" data-hln-ui-control data-variant="primary" data-shape="chamfer" title="添加">
                ＋
              </button>
            </form>
            <ul className="list" data-hln-ui-scroll>
              {activeGoals.map((g, i) => (
                <li
                  key={g.id}
                  className="goal-row"
                  data-hln-motion="item"
                  data-hln-motion-state="enter"
                  data-hln-motion-variant="data-stream"
                  style={motionItem(i)}
                >
                  <button
                    className="check"
                    title="标记完成"
                    onClick={async () => {
                      await setGoalDone(g.id, true);
                      await refresh();
                    }}
                  >
                    ✓
                  </button>
                  <div className="goal-main" onClick={() => setOpenGoalId(g.id)}>
                    <div className="goal-line">
                      <span className="title">{g.title}</span>
                      <span className="nums">
                        {g.progress.done}/{g.progress.total}
                      </span>
                    </div>
                    {meter(g.progress)}
                  </div>
                  <button
                    className="del"
                    title="删除"
                    onClick={async () => {
                      await deleteGoal(g.id);
                      await refresh();
                    }}
                  >
                    ✕
                  </button>
                </li>
              ))}
              {doneGoals.length > 0 && (
                <>
                  <li className="section" onClick={flipPref("taskasion.goalsDone.open", setShowDone)}>
                    <span>已完成 {doneGoals.length}</span>
                    <span className="caret">{showDone ? "▾" : "▸"}</span>
                  </li>
                  {showDone &&
                    doneGoals.map((g, i) => (
                      <li
                        key={g.id}
                        className="goal-row done"
                        data-hln-motion="item"
                        data-hln-motion-state="enter"
                        data-hln-motion-variant="data-stream"
                        style={motionItem(i)}
                      >
                        <button
                          className="check checked"
                          title="重新打开"
                          onClick={async () => {
                            await setGoalDone(g.id, false);
                            await refresh();
                          }}
                        >
                          ✓
                        </button>
                        <div className="goal-main" onClick={() => setOpenGoalId(g.id)}>
                          <div className="goal-line">
                            <span className="title">{g.title}</span>
                            <span className="nums">
                              {g.progress.done}/{g.progress.total}
                            </span>
                          </div>
                          {meter(g.progress)}
                        </div>
                        <button
                          className="del"
                          title="删除"
                          onClick={async () => {
                            await deleteGoal(g.id);
                            await refresh();
                          }}
                        >
                          ✕
                        </button>
                      </li>
                    ))}
                </>
              )}
              {goals.length === 0 && (
                <li className="empty">
                  <span className="empty-code">// NO ENTRY</span>
                  还没有目标,先立一个小目标
                </li>
              )}
            </ul>
          </>
        ) : (
          <>
            <form className="adder" onSubmit={submitTask}>
              <input
                type="text"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="今天做什么？"
                data-hln-ui-field
              />
              <button type="submit" className="add-btn" data-hln-ui-control data-variant="primary" data-shape="chamfer" title="添加">
                ＋
              </button>
              <div className="adder-sub">
                <button
                  type="button"
                  className={`adder-pri${pri ? ` ${pri}` : ""}`}
                  title="优先级:点击切换 无 → P1 → P2 → P3"
                  onClick={() => setPri(PRI_CYCLE[pri] ?? "")}
                  data-hln-ui-control
                >
                  {pri ? pri.toUpperCase() : "PRI"}
                </button>
                <input
                  type="date"
                  value={due}
                  onChange={(e) => setDue(e.target.value)}
                  title="截止日期"
                  data-hln-ui-field
                  className="adder-date"
                />
                {/* ◷ 与时刻框同槽互换:打开=输入框占按钮位(备注框常在,零布局跳动);
                    回车/点外提交并自动规范化,Esc 恢复打开前的时刻。 */}
                {showRemind ? (
                  <WheelTime
                    className="adder-time"
                    title="提醒时刻(框内滚轮上下 ±5 分钟;键入 22:06 / 2206 / 9:5 均可;Esc 取消)"
                    initial={remind}
                    onCommit={(v) => {
                      setRemind(v);
                      setShowRemind(false);
                    }}
                    onCancel={() => {
                      setRemind(remindBeforeEdit.current);
                      setShowRemind(false);
                    }}
                  />
                ) : (
                  <button
                    type="button"
                    className={`adder-clock${remind ? " on" : ""}`}
                    title={remind ? `提醒 ${remind}(点击修改;清空输入并离开 = 不提醒)` : "加一个提醒时刻"}
                    data-hln-ui-control
                    onClick={() => {
                      remindBeforeEdit.current = remind;
                      setShowRemind(true);
                    }}
                  >
                    <span className="clock-glyph">◷</span>
                    {remind && <span className="clock-time">{remind}</span>}
                  </button>
                )}
                <input
                  type="text"
                  className="adder-note"
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  placeholder="备注"
                  data-hln-ui-field
                />
              </div>
            </form>
            <ul className="list" data-hln-ui-scroll>
              {shown.length > 0 && (
                <li className="section" onClick={flipPref("taskasion.tasksPending.open", setShowPending)}>
                  <span>未完成 {shown.length}</span>
                  <span className="caret">{showPending ? "▾" : "▸"}</span>
                </li>
              )}
              {showPending && shown.map(taskRow)}
              {shownDoneToday.length > 0 && (
                <li className="section" onClick={flipPref("taskasion.tasksTodayDone.open", setShowDoneToday)}>
                  <span>今日完成 {shownDoneToday.length}</span>
                  <span className="caret">{showDoneToday ? "▾" : "▸"}</span>
                </li>
              )}
              {showDoneToday && shownDoneToday.map(taskRow)}
              {archiveVisible && shownArchive.length > 0 && (
                <li className="section" onClick={flipPref("taskasion.tasksArchive.open", setShowArchive)}>
                  <span>归档 {shownArchive.length}</span>
                  <span className="caret">{showArchive ? "▾" : "▸"}</span>
                </li>
              )}
              {archiveVisible && showArchive && shownArchive.map(taskRow)}
              {shown.length === 0 &&
                !(showDoneToday && shownDoneToday.length > 0) &&
                !(archiveVisible && showArchive && shownArchive.length > 0) && (
                  <li className="empty">
                    <span className="empty-code">// NO ENTRY</span>
                    {shownDoneToday.length > 0 || (archiveVisible && shownArchive.length > 0)
                      ? "没有进行中的任务"
                      : openGoal
                        ? "该目标下还没有任务"
                        : filter === "today"
                          ? "今天没有安排,去「全部」看看"
                          : "还没有任务,输入后回车添加"}
                  </li>
                )}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}
