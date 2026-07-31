import importlib.util
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest


SCRIPT = Path(__file__).parents[1] / "scripts" / "convert_tasks_to_linear.py"
SPEC = importlib.util.spec_from_file_location("convert_tasks_to_linear", SCRIPT)
assert SPEC and SPEC.loader
CONVERTER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CONVERTER
SPEC.loader.exec_module(CONVERTER)


def task(task_id: str, aliases: list[str], parent: str | None = None):
    return SimpleNamespace(id=task_id, repository_aliases=aliases, parent=parent)


class RepositoryBindingTests(unittest.TestCase):
    def test_normalize_repository_accepts_frontmatter_and_managed_labels(self):
        errors: list[str] = []

        aliases = CONVERTER.normalize_repository_aliases(
            "core", ["area:orchestrator", "repo:core"], "COE-548", errors
        )

        self.assertEqual(aliases, ["core", "core"])
        self.assertEqual(errors, [])


    def test_project_set_requires_one_binding_and_rejects_unknown_aliases(self):
        errors: list[str] = []
        tasks = {
            "PARENT": task("PARENT", ["core"]),
            "MISSING": task("MISSING", [], "PARENT"),
            "MULTIPLE": task("MULTIPLE", ["core", "web"]),
            "UNKNOWN": task("UNKNOWN", ["other"]),
        }

        CONVERTER.validate_repository_bindings(tasks, "project_set", {"core", "web"}, errors)

        self.assertIn("parent task PARENT must not declare a repository binding", errors)
        self.assertIn("terminal task MISSING must declare exactly one repository binding", errors)
        self.assertIn("task MULTIPLE has multiple managed repository bindings: core, web", errors)
        self.assertIn("task UNKNOWN references unknown repository alias other", errors)

    def test_project_set_requires_repository_inventory(self):
        errors: list[str] = []

        CONVERTER.validate_repository_bindings(
            {"TASK": task("TASK", ["core"])}, "project_set", set(), errors
        )

        self.assertIn(
            "project_set packages must declare a non-empty repositoryAliases inventory",
            errors,
        )
        self.assertIn("task TASK references unknown repository alias core", errors)


    def test_legacy_single_allows_unlabelled_task(self):
        errors: list[str] = []

        CONVERTER.validate_repository_bindings(
            {"TASK": task("TASK", [])}, "legacy_single", {"core"}, errors
        )

        self.assertEqual(errors, [])

    def test_linear_conversion_preserves_unmanaged_labels(self):
        source = task("TASK", ["core"])
        source.areas = ["orchestrator"]

        label_ids = CONVERTER.merge_issue_label_ids(
            source,
            [
                {"id": "unmanaged-label", "name": "area:orchestrator"},
                {"id": "old-repository-label", "name": "repo:old"},
            ],
            {"orchestrator": "area-label"},
            {"core": "repository-label"},
        )

        self.assertEqual(
            label_ids,
            ["unmanaged-label", "area-label", "repository-label"],
        )

    def test_linear_conversion_can_clear_the_last_managed_label(self):
        source = task("PARENT", [])
        source.areas = []

        label_ids = CONVERTER.merge_issue_label_ids(
            source,
            [{"id": "old-repository-label", "name": "repo:old"}],
            {},
            {},
        )

        self.assertEqual(label_ids, [])
