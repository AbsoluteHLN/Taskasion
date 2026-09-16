"""本地 REST API(stdlib 实现,零第三方依赖)。

所有变更走 TaskStore,身份取 X-Taskasion-Actor 头(缺省 human),全部入审计。
"""

from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from .audit import Audit
from .goals import GoalStore
from .store import TaskStore

VERSION = "1.0.1-aStart"


def default_data_dir() -> Path:
    env = os.environ.get("TASKASION_DATA_DIR")
    if env:
        return Path(env)
    return Path.home() / ".taskasion"


class _Handler(BaseHTTPRequestHandler):
    server_version = f"TaskasionCore/{VERSION}"

    # ---- helpers ----

    @property
    def store(self) -> TaskStore:
        return self.server.store  # type: ignore[attr-defined]

    @property
    def goals(self):  # noqa: ANN201
        return self.server.goals  # type: ignore[attr-defined]

    @property
    def audit(self):  # noqa: ANN201
        return self.server.audit  # type: ignore[attr-defined]

    def _send(self, code: int, payload) -> None:
        body = b"" if payload is None else json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type, X-Taskasion-Actor")
        self.end_headers()
        if body:
            self.wfile.write(body)

    def _actor(self) -> str:
        return self.headers.get("X-Taskasion-Actor") or "human"

    def _body(self) -> dict:
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0:
            return {}
        raw = self.rfile.read(length)
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            # Windows 控制台/curl 在中文代码页下发 GBK
            text = raw.decode("gbk", errors="replace")
        return json.loads(text or "{}")

    def log_message(self, fmt, *args):  # 安静模式:仅错误时 stderr 由上层看
        pass

    # ---- goals helpers ----

    def _goal_dict(self, goal) -> dict:
        """goal.to_dict() 附带关联任务进度(tags 含 goal:<id>)。"""
        tasks = self.store.list(status="all", tag=f"goal:{goal.id}")
        done = sum(1 for t in tasks if t.done)
        data = goal.to_dict()
        data["progress"] = {"total": len(tasks), "done": done}
        return data

    def _goal_route(self, parts: list[str]) -> tuple[str, str] | None:
        """api/goals/{id}[/{action}] → (id, action|'')"""
        if len(parts) in (3, 4) and parts[:2] == ["api", "goals"]:
            return parts[2], parts[3] if len(parts) == 4 else ""
        return None

    # ---- routes ----

    def do_OPTIONS(self):  # noqa: N802
        self._send(204, None)

    def do_GET(self):  # noqa: N802
        url = urlparse(self.path)
        query = parse_qs(url.query)
        try:
            if url.path == "/api/health":
                return self._send(200, {"ok": True, "version": VERSION, "data_dir": str(self.store.data_dir)})
            if url.path == "/api/tasks":
                status = (query.get("status") or ["all"])[0]
                tag = (query.get("tag") or [None])[0]
                return self._send(200, [t.to_dict() for t in self.store.list(status=status, tag=tag)])
            if url.path == "/api/audit":
                n = int((query.get("n") or ["50"])[0])
                return self._send(200, self.audit.tail(n))
            if url.path == "/api/plan/today":
                return self._send(200, self.store.plan_today())
            if url.path == "/api/goals":
                status = (query.get("status") or ["all"])[0]
                return self._send(200, [self._goal_dict(g) for g in self.goals.list(status=status)])
            return self._send(404, {"error": "not_found"})
        except Exception as exc:  # noqa: BLE001
            return self._send(500, {"error": str(exc)})

    def do_POST(self):  # noqa: N802
        url = urlparse(self.path)
        try:
            if url.path == "/api/tasks":
                body = self._body()
                task = self.store.add(
                    title=body.get("title", ""),
                    due=body.get("due"),
                    priority=body.get("priority"),
                    tags=body.get("tags"),
                    note=body.get("note"),
                    source=self._actor(),
                )
                return self._send(201, task.to_dict())
            parts = url.path.strip("/").split("/")  # api/tasks/{id}/{action}
            if len(parts) == 4 and parts[:2] == ["api", "tasks"] and parts[3] in ("complete", "reopen"):
                task = self.store.set_done(parts[2], parts[3] == "complete", actor=self._actor())
                return self._send(200, task.to_dict())
            if len(parts) == 2 and parts == ["api", "goals"]:
                body = self._body()
                goal = self.goals.add(title=body.get("title", ""), source=self._actor())
                return self._send(201, self._goal_dict(goal))
            goal_route = self._goal_route(parts)
            if goal_route and goal_route[1] in ("complete", "reopen"):
                goal_id, action = goal_route
                goal = self.goals.set_done(goal_id, action == "complete", actor=self._actor())
                return self._send(200, self._goal_dict(goal))
            return self._send(404, {"error": "not_found"})
        except KeyError as exc:
            return self._send(404, {"error": str(exc)})
        except ValueError as exc:
            return self._send(400, {"error": str(exc)})
        except Exception as exc:  # noqa: BLE001
            return self._send(500, {"error": str(exc)})

    def do_PATCH(self):  # noqa: N802
        parts = urlparse(self.path).path.strip("/").split("/")
        try:
            if len(parts) == 3 and parts[:2] == ["api", "tasks"]:
                task = self.store.update(parts[2], self._body(), actor=self._actor())
                return self._send(200, task.to_dict())
            if len(parts) == 3 and parts[:2] == ["api", "goals"]:
                goal = self.goals.rename(parts[2], self._body().get("title", ""), actor=self._actor())
                return self._send(200, self._goal_dict(goal))
            return self._send(404, {"error": "not_found"})
        except KeyError as exc:
            return self._send(404, {"error": str(exc)})
        except ValueError as exc:
            return self._send(400, {"error": str(exc)})

    def do_DELETE(self):  # noqa: N802
        parts = urlparse(self.path).path.strip("/").split("/")
        try:
            if len(parts) == 3 and parts[:2] == ["api", "tasks"]:
                self.store.delete(parts[2], actor=self._actor())
                return self._send(204, None)
            if len(parts) == 3 and parts[:2] == ["api", "goals"]:
                self.goals.delete(parts[2], actor=self._actor())
                return self._send(204, None)
            return self._send(404, {"error": "not_found"})
        except KeyError as exc:
            return self._send(404, {"error": str(exc)})


def serve(host: str = "127.0.0.1", port: int = 14411, data_dir: str | Path | None = None) -> None:
    data_path = Path(data_dir) if data_dir else default_data_dir()
    audit = Audit(data_path / "audit.jsonl")
    store = TaskStore(data_path, audit=audit)
    goals = GoalStore(data_path, audit=audit)
    server = ThreadingHTTPServer((host, port), _Handler)
    server.store = store  # type: ignore[attr-defined]
    server.goals = goals  # type: ignore[attr-defined]
    server.audit = audit  # type: ignore[attr-defined]
    print(f"taskasion-core {VERSION} listening on http://{host}:{port}  data={data_path}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
