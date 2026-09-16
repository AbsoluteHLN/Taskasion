r"""Taskasion 数据模型与 todo.md 行级序列化。

真相源行格式(人可读的 Markdown checkbox,行尾 HTML 注释为机器元数据):

    - [ ] 交季度报告 <!-- id:a1b2c3d4 due:2026-09-18 pri:p1 tags:work src:human created:2026-09-14T12:00:00 -->

键值以空格分隔,空字段直接省略;备注 note 可含空格,写成引号包裹的
note:"多词备注"(内部 " 和 \ 需转义),其余 value 内不允许空格(tags 用逗号)。
"""

from __future__ import annotations

import re
import uuid
from dataclasses import asdict, dataclass, field

CHECKBOX_RE = re.compile(r"^\s*- \[(?P<mark>[ xX])\] (?P<body>.*)$")
META_RE = re.compile(r"<!--(?P<meta>.*?)-->\s*$")
META_KEYS = ("id", "due", "pri", "tags", "src", "created", "done", "note")
# 引号包裹的 note 值(允许空格);捕获组 1 是转义前的内容
NOTE_QUOTED_RE = re.compile(r'note:"((?:[^"\\]|\\.)*)"')


def new_id() -> str:
    return uuid.uuid4().hex[:8]


@dataclass
class Task:
    id: str
    title: str
    done: bool = False
    due: str | None = None
    priority: str | None = None
    tags: list[str] = field(default_factory=list)
    source: str = "human"
    created: str = ""
    done_at: str | None = None
    note: str | None = None

    def to_dict(self) -> dict:
        return asdict(self)


def _parse_meta(raw: str) -> dict[str, str]:
    meta: dict[str, str] = {}
    # 先摘出引号包裹的 note(可含空格),剩余 token 按空白切分
    def _take_note(m: re.Match) -> str:
        meta["note"] = re.sub(r"\\(.)", r"\1", m.group(1))
        return " "

    raw = NOTE_QUOTED_RE.sub(_take_note, raw)
    for token in raw.split():
        key, sep, value = token.partition(":")
        if sep and key in META_KEYS and value:
            meta[key] = value
    return meta


def parse_line(line: str) -> Task | None:
    match = CHECKBOX_RE.match(line.rstrip())
    if match is None:
        return None
    body = match.group("body")
    meta: dict[str, str] = {}
    meta_match = META_RE.search(body)
    if meta_match:
        meta = _parse_meta(meta_match.group("meta"))
        title = body[: meta_match.start()].rstrip()
    else:
        title = body
    tags = [t for t in meta.get("tags", "").split(",") if t]
    return Task(
        id=meta.get("id", ""),
        title=title.strip() or "(未命名)",
        done=match.group("mark").lower() == "x",
        due=meta.get("due") or None,
        priority=meta.get("pri") or None,
        tags=tags,
        source=meta.get("src") or "human",
        created=meta.get("created", ""),
        done_at=meta.get("done") or None,
        note=meta.get("note") or None,
    )


def _escape_note(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def render_line(task: Task) -> str:
    parts = [f"id:{task.id}"]
    if task.due:
        parts.append(f"due:{task.due}")
    if task.priority:
        parts.append(f"pri:{task.priority}")
    if task.tags:
        parts.append("tags:" + ",".join(task.tags))
    parts.append(f"src:{task.source}")
    if task.created:
        parts.append(f"created:{task.created}")
    if task.done_at:
        parts.append(f"done:{task.done_at}")
    if task.note:
        parts.append(f'note:"{_escape_note(task.note)}"')
    mark = "x" if task.done else " "
    return f"- [{mark}] {task.title} <!-- {' '.join(parts)} -->"
