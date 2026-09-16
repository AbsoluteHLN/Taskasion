import tempfile
import unittest
from pathlib import Path

from taskasion_core.audit import Audit
from taskasion_core.goals import GoalStore
from taskasion_core.store import TaskStore


class GoalsTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.data_dir = Path(self._tmp.name)
        self.audit = Audit(self.data_dir / "audit.jsonl")
        self.goals = GoalStore(self.data_dir, audit=self.audit)

    def tearDown(self):
        self._tmp.cleanup()

    def test_add_and_reload(self):
        goal = self.goals.add("三个月跑通产品化")
        other = GoalStore(self.data_dir)  # 模拟重启:从文件恢复
        loaded = other.list()
        self.assertEqual([g.title for g in loaded], ["三个月跑通产品化"])
        self.assertEqual(loaded[0].id, goal.id)

    def test_rename_complete_delete(self):
        goal = self.goals.add("健身计划")
        self.goals.rename(goal.id, "年度健身计划")
        self.assertEqual(self.goals.list()[0].title, "年度健身计划")
        self.goals.set_done(goal.id, True)
        self.assertEqual([g.id for g in self.goals.list("done")], [goal.id])
        self.assertEqual(self.goals.list("todo"), [])
        self.goals.delete(goal.id)
        self.assertEqual(self.goals.list(), [])

    def test_external_handwritten_goal_gets_id(self):
        p = self.goals.goals_path
        p.write_text(
            "# Taskasion Goals\n\n- [ ] 手写目标 <!-- created:2026-09-15T08:00:00 -->\n",
            encoding="utf-8",
        )
        self.goals.reload_if_changed()
        loaded = self.goals.list()
        self.assertEqual(len(loaded), 1)
        self.assertTrue(loaded[0].id)  # Core 自动补 id 并回写
        self.assertIn(f"id:{loaded[0].id}", p.read_text(encoding="utf-8"))

    def test_audit_records_goal_actions(self):
        goal = self.goals.add("学 Rust", source="agent:mcp")
        self.goals.set_done(goal.id, True, actor="human")
        actions = [(r["actor"], r["action"]) for r in self.audit.tail(10)]
        self.assertIn(("agent:mcp", "goal_add"), actions)
        self.assertIn(("human", "goal_complete"), actions)

    def test_linked_task_progress(self):
        """进度统计在 server 层;此处验证任务侧 goal:<id> 标签过滤。"""
        goal = self.goals.add("写书")
        store = TaskStore(self.data_dir, audit=self.audit)
        t1 = store.add("写第一章", tags=[f"goal:{goal.id}"])
        store.add("写第二章", tags=[f"goal:{goal.id}"])
        store.add("无关任务")
        linked = store.list(tag=f"goal:{goal.id}")
        self.assertEqual(len(linked), 2)
        store.set_done(t1.id, True)
        self.assertEqual(len(store.list(tag=f"goal:{goal.id}", status="done")), 1)
        self.assertEqual(len(store.list(tag=f"goal:{goal.id}", status="todo")), 1)


if __name__ == "__main__":
    unittest.main()
