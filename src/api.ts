const RAW_BASE = (import.meta.env.VITE_CORE_URL as string | undefined) ?? "http://127.0.0.1:14411";
const BASE = `${RAW_BASE.replace(/\/$/, "")}/api`;

export interface Task {
  id: string;
  title: string;
  done: boolean;
  due: string | null;
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
  priority?: string | null;
  tags?: string[];
  note?: string | null;
}) => req<Task>("/tasks", { method: "POST", body: JSON.stringify(body) });
export const updateTask = (id: string, body: { note?: string | null; priority?: string | null }) =>
  req<Task>(`/tasks/${id}`, { method: "PATCH", body: JSON.stringify(body) });
export const setDone = (id: string, done: boolean) =>
  req<Task>(`/tasks/${id}/${done ? "complete" : "reopen"}`, { method: "POST" });
export const deleteTask = (id: string) => req<void>(`/tasks/${id}`, { method: "DELETE" });

export const fetchGoals = () => req<Goal[]>("/goals?status=all");
export const addGoal = (title: string) => req<Goal>("/goals", { method: "POST", body: JSON.stringify({ title }) });
export const setGoalDone = (id: string, done: boolean) =>
  req<Goal>(`/goals/${id}/${done ? "complete" : "reopen"}`, { method: "POST" });
export const deleteGoal = (id: string) => req<void>(`/goals/${id}`, { method: "DELETE" });

export function todayLocal(): string {
  const d = new Date();
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 10);
}
