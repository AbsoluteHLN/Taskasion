import tempfile
import unittest
from pathlib import Path

from taskasion_core.audit import Audit
from taskasion_core.store import TaskStore


class StoreTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.data_dir = Path(self._tmp.name)
        self.audit = Audit(self.data_dir / "audit.jsonl")
        self.store = TaskStore(self.data_dir, audit=self.audit)

    def tearDown(self):
        self._tmp.cleanup()

    def test_add_and_reload(self):
        task = self.store.add("交季度报告", due="2026-09-18", priority="p1", tags=["work"])
        other = TaskStore(self.data_dir)  # 模拟重启:从文件恢复
        self.assertEqual([t.title for t in other.list()], ["交季度报告"])
        loaded = other.list()[0]
        self.assertEqual(loaded.id, task.id)
        self.assertEqual(loaded.due, "2026-09-18")
        self.assertEqual(loaded.tags, ["work"])

    def test_complete_and_reopen(self):
        task = self.store.add("买菜")
        self.store.set_done(task.id, True)
        self.assertTrue(self.store.list(status="done")[0].done)
        self.assertEqual(self.store.list(status="todo"), [])
        raw = self.store.todo_path.read_text(encoding="utf-8")
        self.assertIn("- [x] 买菜", raw)
        self.store.set_done(task.id, False)
        self.assertEqual(len(self.store.list(status="todo")), 1)

    def test_update_and_delete(self):
        task = self.store.add("写周报", tags=["work"])
        self.store.update(task.id, {"title": "写周报 v2", "due": "2026-09-15"})
        got = self.store.get(task.id)
        assert got
        self.assertEqual(got.title, "写周报 v2")
        self.assertEqual(got.due, "2026-09-15")
        self.store.delete(task.id)
        self.assertIsNone(self.store.get(task.id))

    def test_external_edit_is_picked_up(self):
        task = self.store.add("已有任务")
        # 模拟 Agent/人直接改文件
        raw = self.store.todo_path.read_text(encoding="utf-8")
        self.store.todo_path.write_text(raw + "\n- [ ] 外部手写的任务\n", encoding="utf-8")
        changed = self.store.reload_if_changed()
        self.assertTrue(changed)
        titles = [t.title for t in self.store.list()]
        self.assertIn("外部手写的任务", titles)
        # 手写行获得稳定 id:再次外部编辑后 id 不变
        ext_id = next(t.id for t in self.store.list() if t.title == "外部手写的任务")
        self.store.reload_if_changed()  # mtime 未变,不应重载
        self.assertEqual(self.store.get(ext_id).title, "外部手写的任务")  # type: ignore[union-attr]

    def test_audit_records_agent_and_external(self):
        self.store.add("a", source="agent:test")
        self.store.add("b")
        raw = self.store.todo_path.read_text(encoding="utf-8")
        self.store.todo_path.write_text(raw + "\n- [ ] c\n", encoding="utf-8")
        self.store.reload_if_changed()
        actions = [(e["actor"], e["action"]) for e in self.audit.tail(10)]
        self.assertIn(("agent:test", "add"), actions)
        self.assertIn(("external", "external_edit"), actions)

    def test_plan_today(self):
        from datetime import date, timedelta

        today = date.today().isoformat()
        overdue_date = (date.today() - timedelta(days=1)).isoformat()
        self.store.add("过期了", due=overdue_date)
        self.store.add("今天做", due=today)
        plan = self.store.plan_today()
        self.assertEqual([t["title"] for t in plan["overdue"]], ["过期了"])
        self.assertEqual([t["title"] for t in plan["today_tasks"]], ["今天做"])


if __name__ == "__main__":
    unittest.main()
