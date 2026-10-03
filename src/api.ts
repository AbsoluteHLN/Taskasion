const RAW_BASE = (import.meta.env.VITE_CORE_URL as string | undefined) ?? "http://127.0.0.1:14411";
// Integration API v1:前端与外部集成(Bot Bridge / 脚本)走同一条稳定契约。
// 旧的 /api/* 仍然可用,此处显式使用 /api/v1 表达"我们是契约的消费方之一"。
const BASE = `${RAW_BASE.replace(/\/$/, "")}/api/v1`;

export interface Task {
  id: string;
  title: string;
  done: boolean;
  due: string | null;
  /** HH:MM(24 小时制),可选。内部存为保留标签 _remind:HH:MM,不写进 tags。 */
  remind_time: string | null;
  priority: string | null;
  tags: string[];
  source: string;
  created: string;
  done_at: string | null;
  note: string | null;
}

export interface Goal {
  id: string;
  title: string;
  done: boolean;
  due: string | null;
  remind_time: string | null;
  priority: string | null;
  tags: string[];
  source: string;
  created: string;
  done_at: string | null;
  progress: { total: number; done: number };
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", "X-Taskasion-Actor": "human", ...(init?.headers ?? {}) },
  });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

export const fetchTasks = () => req<Task[]>("/tasks?status=all");
export const addTask = (body: {
  title: string;
  due?: string | null;
  remind_time?: string | null;
  priority?: string | null;
  tags?: string[];
  note?: string | null;
}) => req<Task>("/tasks", { method: "POST", body: JSON.stringify(body) });
export const updateTask = (
  id: string,
  body: {
    title?: string;
    due?: string | null;
    remind_time?: string | null;
    note?: string | null;
    priority?: string | null;
    tags?: string[];
  },
) => req<Task>(`/tasks/${id}`, { method: "PATCH", body: JSON.stringify(body) });
export const setDone = (id: string, done: boolean) =>
  req<Task>(`/tasks/${id}/${done ? "complete" : "reopen"}`, { method: "POST" });
export const deleteTask = (id: string) => req<void>(`/tasks/${id}`, { method: "DELETE" });

export const fetchGoals = () => req<Goal[]>("/goals?status=all");
export const addGoal = (title: string) => req<Goal>("/goals", { method: "POST", body: JSON.stringify({ title }) });
export const setGoalDone = (id: string, done: boolean) =>
  req<Goal>(`/goals/${id}/${done ? "complete" : "reopen"}`, { method: "POST" });
export const deleteGoal = (id: string) => req<void>(`/goals/${id}`, { method: "DELETE" });

/** 本地时区的 YYYY-MM-DD。 */
export function todayLocal(): string {
  const d = new Date();
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 10);
}

/** 明天(本地时区)的 YYYY-MM-DD。 */
export function tomorrowLocal(): string {
  const d = new Date();
  return new Date(d.getTime() + 86400000 - d.getTimezoneOffset() * 60000).toISOString().slice(0, 10);
}

/** 当前本地 HH:MM。 */
export function nowLocalTime(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

