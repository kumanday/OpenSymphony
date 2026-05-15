#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["PyYAML>=6.0.2"]
# ///
"""Validate and publish OpenSymphony task packages to Linear."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict, deque
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import yaml


REQUIRED_FRONTMATTER = [
    "id",
    "title",
    "milestone",
    "priority",
    "estimate",
    "blockedBy",
    "blocks",
    "parent",
]
REQUIRED_SECTIONS = [
    "Summary",
    "Scope",
    "Deliverables",
    "Acceptance Criteria",
    "Test Plan",
    "Context",
    "Definition of Ready",
]
PRIORITY_NAMES = {
    0: "No priority",
    1: "Urgent",
    2: "High",
    3: "Normal",
    4: "Low",
}


@dataclass(frozen=True)
class ManifestTask:
    id: str
    file: str


@dataclass
class Task:
    id: str
    file: str
    path: Path
    title: str
    milestone: str
    priority: int
    estimate: int
    blocked_by: list[str]
    blocks: list[str]
    parent: str | None
    body: str


@dataclass
class Package:
    manifest_path: Path
    repo_root: Path
    planning_wave: str
    tasks_dir: str
    milestones: list[str]
    manifest_tasks: list[ManifestTask]
    tasks: dict[str, Task]
    waves: list[list[str]]


class ValidationError(Exception):
    """Raised when the task package is invalid."""


class LinearError(Exception):
    """Raised when a Linear GraphQL operation fails."""


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate, preview, or publish a docs/tasks task package."
    )
    parser.add_argument("--manifest", default=None, help="Path to task-package.yaml.")
    parser.add_argument("--repo-root", default=None, help="Repository root.")
    subcommands = parser.add_subparsers(dest="command", required=True)

    validate_parser = subcommands.add_parser("validate", help="Validate the task package locally.")
    add_common_args(validate_parser)
    dry_run_parser = subcommands.add_parser("dry-run", help="Print conversion waves without Linear writes.")
    add_common_args(dry_run_parser)

    apply_parser = subcommands.add_parser("apply", help="Publish the task package to Linear.")
    add_common_args(apply_parser)
    apply_parser.add_argument("--project-slug", required=True, help="Linear project slugId.")
    apply_parser.add_argument("--team-key", help="Linear team key when a project has multiple teams.")
    apply_parser.add_argument(
        "--publish",
        default=None,
        help="Publish mapping path. Defaults to <tasksDir>/linear-publish.yaml.",
    )
    apply_parser.add_argument(
        "--no-project-overview",
        action="store_true",
        help="Skip updating the Linear project overview.",
    )

    args = parser.parse_args()
    repo_root = Path(args.repo_root or ".").resolve()
    manifest_path = resolve_path(repo_root, args.manifest or "docs/tasks/task-package.yaml")

    try:
        package = load_package(repo_root, manifest_path)
        if args.command == "validate":
            print_validation_summary(package)
            return 0
        if args.command == "dry-run":
            print_dry_run(package)
            return 0
        if args.command == "apply":
            publish_path = resolve_publish_path(repo_root, package, args.publish)
            apply_to_linear(
                package=package,
                project_slug=args.project_slug,
                team_key=args.team_key,
                publish_path=publish_path,
                update_project_overview=not args.no_project_overview,
            )
            return 0
    except (ValidationError, LinearError) as error:
        print(str(error), file=sys.stderr)
        return 1

    raise AssertionError(f"unhandled command {args.command}")


def add_common_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--manifest", default=None, help="Path to task-package.yaml.")
    parser.add_argument("--repo-root", default=None, help="Repository root.")


def resolve_path(repo_root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else repo_root / path


def resolve_publish_path(repo_root: Path, package: Package, value: str | None) -> Path:
    if value:
        return resolve_path(repo_root, value)
    return repo_root / package.tasks_dir / "linear-publish.yaml"


def load_package(repo_root: Path, manifest_path: Path) -> Package:
    errors: list[str] = []
    manifest = load_yaml_file(manifest_path, errors, "manifest")
    if not isinstance(manifest, dict):
        raise ValidationError("task package manifest must be a YAML mapping")

    planning_wave = require_non_empty_string(manifest, "planningWave", errors)
    tasks_dir = require_non_empty_string(manifest, "tasksDir", errors)
    milestones = normalize_milestones(manifest.get("milestones"), errors)
    manifest_tasks = normalize_manifest_tasks(manifest.get("tasks"), errors)

    tasks: dict[str, Task] = {}
    for manifest_task in manifest_tasks:
        path = resolve_path(repo_root, manifest_task.file)
        task = load_task(path, manifest_task, milestones, errors)
        if task and task.id in tasks:
            errors.append(f"duplicate task id {task.id}")
        elif task:
            tasks[task.id] = task

    validate_manifest_references(manifest_tasks, tasks, milestones, errors)
    validate_task_graph(tasks, errors)

    if errors:
        raise ValidationError(render_errors("Task package validation failed", errors))

    waves = dependency_waves(tasks)
    return Package(
        manifest_path=manifest_path,
        repo_root=repo_root,
        planning_wave=planning_wave,
        tasks_dir=tasks_dir,
        milestones=milestones,
        manifest_tasks=manifest_tasks,
        tasks=tasks,
        waves=waves,
    )


def load_yaml_file(path: Path, errors: list[str], label: str) -> Any:
    if not path.is_file():
        errors.append(f"{label} file does not exist: {path}")
        return None
    try:
        return yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as error:
        errors.append(f"{label} YAML parse failed in {path}: {error}")
    except OSError as error:
        errors.append(f"failed to read {label} file {path}: {error}")
    return None


def require_non_empty_string(data: dict[str, Any], key: str, errors: list[str]) -> str:
    value = data.get(key)
    if isinstance(value, str) and value.strip():
        return value.strip()
    errors.append(f"manifest field {key} must be a non-empty string")
    return ""


def normalize_milestones(value: Any, errors: list[str]) -> list[str]:
    if not isinstance(value, list) or not value:
        errors.append("manifest field milestones must be a non-empty list")
        return []

    milestones: list[str] = []
    for index, item in enumerate(value):
        name = normalize_milestone_name(item)
        if not name:
            errors.append(f"milestones[{index}] must be a non-empty string")
            continue
        milestones.append(name)

    duplicates = sorted(name for name, count in counts(milestones).items() if count > 1)
    for duplicate in duplicates:
        errors.append(f"duplicate milestone {duplicate}")
    return milestones


def normalize_milestone_name(value: Any) -> str | None:
    if isinstance(value, str):
        return value.strip() or None
    if isinstance(value, dict) and len(value) == 1:
        key, item = next(iter(value.items()))
        if isinstance(key, str) and isinstance(item, str):
            return f"{key}: {item}".strip()
    return None


def normalize_manifest_tasks(value: Any, errors: list[str]) -> list[ManifestTask]:
    if not isinstance(value, list) or not value:
        errors.append("manifest field tasks must be a non-empty list")
        return []

    tasks: list[ManifestTask] = []
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            errors.append(f"tasks[{index}] must be a mapping")
            continue
        task_id = item.get("id")
        file_path = item.get("file")
        if not isinstance(task_id, str) or not task_id.strip():
            errors.append(f"tasks[{index}].id must be a non-empty string")
            continue
        if not isinstance(file_path, str) or not file_path.strip():
            errors.append(f"tasks[{index}].file must be a non-empty string")
            continue
        tasks.append(ManifestTask(id=task_id.strip(), file=file_path.strip()))

    for duplicate in sorted(name for name, count in counts([task.id for task in tasks]).items() if count > 1):
        errors.append(f"duplicate manifest task id {duplicate}")
    for duplicate in sorted(name for name, count in counts([task.file for task in tasks]).items() if count > 1):
        errors.append(f"duplicate manifest task file {duplicate}")
    return tasks


def load_task(
    path: Path,
    manifest_task: ManifestTask,
    milestones: list[str],
    errors: list[str],
) -> Task | None:
    if not path.is_file():
        errors.append(f"task {manifest_task.id} file does not exist: {manifest_task.file}")
        return None

    text = path.read_text(encoding="utf-8")
    match = re.match(r"\A---\n(.*?)\n---\n?", text, re.DOTALL)
    if not match:
        errors.append(f"task {manifest_task.id} is missing YAML frontmatter: {manifest_task.file}")
        return None

    try:
        frontmatter = yaml.safe_load(match.group(1))
    except yaml.YAMLError as error:
        errors.append(f"task {manifest_task.id} frontmatter YAML parse failed: {error}")
        return None
    if not isinstance(frontmatter, dict):
        errors.append(f"task {manifest_task.id} frontmatter must be a YAML mapping")
        return None

    for key in REQUIRED_FRONTMATTER:
        if key not in frontmatter:
            errors.append(f"task {manifest_task.id} is missing frontmatter field {key}")

    task_id = frontmatter.get("id")
    title = frontmatter.get("title")
    milestone = frontmatter.get("milestone")
    priority = frontmatter.get("priority")
    estimate = frontmatter.get("estimate")
    blocked_by = frontmatter.get("blockedBy")
    blocks = frontmatter.get("blocks")
    parent = frontmatter.get("parent")

    if task_id != manifest_task.id:
        errors.append(
            f"task file {manifest_task.file} has id {task_id!r}, expected {manifest_task.id!r}"
        )
    if not isinstance(title, str) or not title.strip():
        errors.append(f"task {manifest_task.id} title must be a non-empty string")
    if not isinstance(milestone, str) or milestone not in milestones:
        errors.append(f"task {manifest_task.id} milestone must match a manifest milestone")
    if not isinstance(priority, int) or priority not in PRIORITY_NAMES:
        errors.append(f"task {manifest_task.id} priority must be an integer from 0 through 4")
    if not isinstance(estimate, int) or estimate < 0:
        errors.append(f"task {manifest_task.id} estimate must be a non-negative integer")
    if not is_string_list(blocked_by):
        errors.append(f"task {manifest_task.id} blockedBy must be a list of task IDs")
        blocked_by = []
    if not is_string_list(blocks):
        errors.append(f"task {manifest_task.id} blocks must be a list of task IDs")
        blocks = []
    if parent is not None and (not isinstance(parent, str) or not parent.strip()):
        errors.append(f"task {manifest_task.id} parent must be null or a task ID")
        parent = None

    body = text[match.end() :].strip()
    validate_sections(manifest_task.id, manifest_task.file, body, errors)

    return Task(
        id=manifest_task.id,
        file=manifest_task.file,
        path=path,
        title=title.strip() if isinstance(title, str) else manifest_task.id,
        milestone=milestone if isinstance(milestone, str) else "",
        priority=priority if isinstance(priority, int) else 3,
        estimate=estimate if isinstance(estimate, int) else 0,
        blocked_by=list(blocked_by) if is_string_list(blocked_by) else [],
        blocks=list(blocks) if is_string_list(blocks) else [],
        parent=parent.strip() if isinstance(parent, str) else None,
        body=body,
    )


def validate_sections(task_id: str, file_path: str, body: str, errors: list[str]) -> None:
    headings = set(re.findall(r"^##\s+(.+?)\s*$", body, re.MULTILINE))
    for section in REQUIRED_SECTIONS:
        if section not in headings:
            errors.append(f"task {task_id} is missing section ## {section} in {file_path}")


def validate_manifest_references(
    manifest_tasks: list[ManifestTask],
    tasks: dict[str, Task],
    milestones: list[str],
    errors: list[str],
) -> None:
    manifest_ids = {task.id for task in manifest_tasks}
    loaded_ids = set(tasks)
    for task_id in sorted(manifest_ids - loaded_ids):
        errors.append(f"manifest task {task_id} could not be loaded")

    for task in tasks.values():
        if task.milestone not in milestones:
            errors.append(f"task {task.id} milestone is not declared in the manifest")
        for dependency in task.blocked_by:
            if dependency not in manifest_ids:
                errors.append(f"task {task.id} blockedBy references unknown task {dependency}")
            if dependency == task.id:
                errors.append(f"task {task.id} cannot be blocked by itself")
        for blocked in task.blocks:
            if blocked not in manifest_ids:
                errors.append(f"task {task.id} blocks references unknown task {blocked}")
            if blocked == task.id:
                errors.append(f"task {task.id} cannot block itself")
        if task.parent:
            if task.parent not in manifest_ids:
                errors.append(f"task {task.id} parent references unknown task {task.parent}")
            if task.parent == task.id:
                errors.append(f"task {task.id} cannot be its own parent")
            if task.parent in task.blocked_by or task.parent in task.blocks:
                errors.append(f"task {task.id} must not add blocker metadata to its parent")


def validate_task_graph(tasks: dict[str, Task], errors: list[str]) -> None:
    parent_graph: dict[str, list[str]] = defaultdict(list)
    for task in tasks.values():
        if task.parent:
            parent_graph[task.parent].append(task.id)
    if has_cycle({task_id: children for task_id, children in parent_graph.items()}):
        errors.append("parent relationships contain a cycle")

    dependency_graph = {task.id: list(task.blocked_by) for task in tasks.values()}
    cycle = dependency_cycle(dependency_graph)
    if cycle:
        errors.append(f"blockedBy dependencies contain a cycle: {' -> '.join(cycle)}")

    creation_graph = {
        task.id: list(task.blocked_by) + ([task.parent] if task.parent else [])
        for task in tasks.values()
    }
    cycle = dependency_cycle(creation_graph)
    if cycle:
        errors.append(f"creation dependencies contain a cycle: {' -> '.join(cycle)}")


def dependency_waves(tasks: dict[str, Task]) -> list[list[str]]:
    remaining = set(tasks)
    created: set[str] = set()
    waves: list[list[str]] = []

    while remaining:
        wave = sorted(
            task_id
            for task_id in remaining
            if all(dep in created for dep in tasks[task_id].blocked_by)
            and (tasks[task_id].parent is None or tasks[task_id].parent in created)
        )
        if not wave:
            raise ValidationError("unable to compute dependency waves")
        waves.append(wave)
        created.update(wave)
        remaining.difference_update(wave)
    return waves


def is_string_list(value: Any) -> bool:
    return isinstance(value, list) and all(isinstance(item, str) and item.strip() for item in value)


def counts(values: list[str]) -> dict[str, int]:
    result: dict[str, int] = defaultdict(int)
    for value in values:
        result[value] += 1
    return result


def has_cycle(graph: dict[str, list[str]]) -> bool:
    return bool(dependency_cycle(graph))


def dependency_cycle(graph: dict[str, list[str]]) -> list[str]:
    visiting: set[str] = set()
    visited: set[str] = set()
    path: list[str] = []

    def visit(node: str) -> list[str]:
        if node in visiting:
            index = path.index(node)
            return path[index:] + [node]
        if node in visited:
            return []
        visiting.add(node)
        path.append(node)
        for neighbor in graph.get(node, []):
            cycle = visit(neighbor)
            if cycle:
                return cycle
        visiting.remove(node)
        visited.add(node)
        path.pop()
        return []

    for node in sorted(graph):
        cycle = visit(node)
        if cycle:
            return cycle
    return []


def render_errors(title: str, errors: list[str]) -> str:
    lines = [title + ":"]
    lines.extend(f"- {error}" for error in errors)
    return "\n".join(lines)


def print_validation_summary(package: Package) -> None:
    print(f"planningWave: {package.planning_wave}")
    print(f"milestones: {len(package.milestones)}")
    print(f"tasks: {len(package.tasks)}")
    print(f"waves: {len(package.waves)}")
    print("validation: ok")


def print_dry_run(package: Package) -> None:
    print_validation_summary(package)
    print()
    print("Milestones:")
    for milestone in package.milestones:
        count = sum(1 for task in package.tasks.values() if task.milestone == milestone)
        print(f"- {milestone} ({count} task(s))")
    print()
    print("Creation waves:")
    for index, wave in enumerate(package.waves, start=1):
        print(f"- Wave {index}: {', '.join(wave)}")


class LinearClient:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root
        self.helper = repo_root / ".agents/skills/linear/scripts/linear_graphql.py"
        self.queries = repo_root / ".agents/skills/linear/queries"
        if not self.helper.is_file():
            raise LinearError(f"Linear helper not found: {self.helper}")

    def call(self, query_name: str, variables: dict[str, Any], allow_errors: bool = False) -> dict[str, Any]:
        query_file = self.queries / query_name
        if not query_file.is_file():
            raise LinearError(f"Linear query file not found: {query_file}")
        with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8", delete=False) as temp:
            json.dump(variables, temp)
            temp_path = temp.name
        try:
            result = subprocess.run(
                [
                    "python3",
                    str(self.helper),
                    "--query-file",
                    str(query_file),
                    "--variables-file",
                    temp_path,
                ],
                cwd=self.repo_root,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
        finally:
            Path(temp_path).unlink(missing_ok=True)

        data = json.loads(result.stdout) if result.stdout.strip().startswith("{") else None
        if result.returncode != 0 and not allow_errors:
            detail = result.stdout.strip() or result.stderr.strip()
            raise LinearError(f"Linear GraphQL call failed for {query_name}: {detail}")
        if data is None:
            raise LinearError(f"Linear GraphQL call returned non-JSON output for {query_name}")
        if data.get("errors") and not allow_errors:
            raise LinearError(f"Linear GraphQL errors for {query_name}: {json.dumps(data['errors'], indent=2)}")
        return data


def apply_to_linear(
    package: Package,
    project_slug: str,
    team_key: str | None,
    publish_path: Path,
    update_project_overview: bool,
) -> None:
    client = LinearClient(package.repo_root)
    state = load_project_state(client, project_slug)
    project = state["project"]
    team = select_team(project, team_key)
    publish = load_publish_file(publish_path)

    milestone_map = ensure_milestones(client, package, project)
    issue_map = ensure_issues(client, package, project, team, milestone_map, publish)
    apply_blockers(client, package, issue_map)
    rewrite_issue_bodies(client, package, milestone_map, issue_map, project_slug)

    if update_project_overview:
        update_overview(client, package, issue_map, project)

    write_publish_file(publish_path, package, project_slug, milestone_map, issue_map)
    print(f"published tasks: {len(issue_map)}")
    print(f"publish mapping: {publish_path}")


def load_project_state(client: LinearClient, project_slug: str) -> dict[str, Any]:
    data = client.call("project_planning_state.graphql", {"slug": project_slug})
    nodes = data.get("data", {}).get("projects", {}).get("nodes", [])
    if not nodes:
        raise LinearError(f"Linear project not found for slug {project_slug}")
    return {"project": nodes[0]}


def select_team(project: dict[str, Any], team_key: str | None) -> dict[str, Any]:
    teams = project.get("teams", {}).get("nodes", [])
    if team_key:
        for team in teams:
            if team.get("key") == team_key:
                return team
        raise LinearError(f"project has no team with key {team_key}")
    if len(teams) == 1:
        return teams[0]
    keys = ", ".join(team.get("key", "") for team in teams)
    raise LinearError(f"project has multiple teams; pass --team-key. Available: {keys}")


def load_publish_file(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {}
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    return data if isinstance(data, dict) else {}


def ensure_milestones(
    client: LinearClient,
    package: Package,
    project: dict[str, Any],
) -> dict[str, dict[str, Any]]:
    existing = {
        milestone["name"]: milestone
        for milestone in project.get("projectMilestones", {}).get("nodes", [])
    }
    milestone_map: dict[str, dict[str, Any]] = {}
    for name in package.milestones:
        if name in existing:
            milestone_map[name] = existing[name]
            continue
        data = client.call(
            "project_milestone_create.graphql",
            {"input": {"projectId": project["id"], "name": name}},
        )
        milestone = data["data"]["projectMilestoneCreate"]["projectMilestone"]
        milestone_map[name] = milestone
        print(f"created milestone: {name}")
    return milestone_map


def ensure_issues(
    client: LinearClient,
    package: Package,
    project: dict[str, Any],
    team: dict[str, Any],
    milestone_map: dict[str, dict[str, Any]],
    publish: dict[str, Any],
) -> dict[str, dict[str, Any]]:
    existing_by_provenance = issues_by_provenance(project, package.planning_wave)
    publish_tasks = publish.get("tasks", {}) if isinstance(publish.get("tasks"), dict) else {}
    issue_map: dict[str, dict[str, Any]] = {}

    for wave in package.waves:
        for task_id in wave:
            task = package.tasks[task_id]
            mapped = publish_tasks.get(task_id, {}) if isinstance(publish_tasks.get(task_id), dict) else {}
            existing = None
            if mapped.get("issueId"):
                existing = {"id": mapped["issueId"], "identifier": mapped.get("issue"), "url": mapped.get("url")}
            elif task_id in existing_by_provenance:
                existing = existing_by_provenance[task_id]

            body = issue_body(package, task, issue_map=None)
            input_data: dict[str, Any] = {
                "teamId": team["id"],
                "projectId": project["id"],
                "projectMilestoneId": milestone_map[task.milestone]["id"],
                "title": task.title,
                "description": body,
                "priority": task.priority,
                "estimate": task.estimate,
            }
            if task.parent:
                input_data["parentId"] = issue_map[task.parent]["id"]

            if existing:
                issue = update_issue(client, existing["id"], input_data)
                print(f"updated issue: {issue['identifier']} {task.title}")
            else:
                issue = create_issue(client, input_data, task.title)
                print(f"created issue: {issue['identifier']} {task.title}")
            issue_map[task_id] = issue
    return issue_map


def issues_by_provenance(project: dict[str, Any], planning_wave: str) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for issue in project.get("issues", {}).get("nodes", []):
        description = issue.get("description") or ""
        wave_match = re.search(r"<!--\s*task-planning-wave:\s*(.*?)\s*-->", description)
        id_match = re.search(r"<!--\s*task-source-id:\s*(.*?)\s*-->", description)
        if wave_match and id_match and wave_match.group(1) == planning_wave:
            result[id_match.group(1)] = issue
    return result


def create_issue(client: LinearClient, input_data: dict[str, Any], title: str) -> dict[str, Any]:
    data = client.call("issue_create.graphql", {"input": input_data}, allow_errors=True)
    if data.get("errors") and "estimate" in json.dumps(data["errors"]).lower():
        retry_input = dict(input_data)
        retry_input.pop("estimate", None)
        data = client.call("issue_create.graphql", {"input": retry_input})
    elif data.get("errors"):
        raise LinearError(f"failed to create issue {title}: {json.dumps(data['errors'], indent=2)}")
    return data["data"]["issueCreate"]["issue"]


def update_issue(client: LinearClient, issue_id: str, input_data: dict[str, Any]) -> dict[str, Any]:
    update_input = dict(input_data)
    update_input.pop("teamId", None)
    data = client.call(
        "issue_update.graphql",
        {"id": issue_id, "input": update_input},
        allow_errors=True,
    )
    if data.get("errors") and "estimate" in json.dumps(data["errors"]).lower():
        retry_input = dict(update_input)
        retry_input.pop("estimate", None)
        data = client.call("issue_update.graphql", {"id": issue_id, "input": retry_input})
    elif data.get("errors"):
        raise LinearError(f"failed to update issue {issue_id}: {json.dumps(data['errors'], indent=2)}")
    return data["data"]["issueUpdate"]["issue"]


def apply_blockers(client: LinearClient, package: Package, issue_map: dict[str, dict[str, Any]]) -> None:
    for task in package.tasks.values():
        for blocker_id in task.blocked_by:
            blocker = issue_map[blocker_id]
            blocked = issue_map[task.id]
            data = client.call(
                "issue_relation_create.graphql",
                {
                    "input": {
                        "issueId": blocker["id"],
                        "type": "blocks",
                        "relatedIssueId": blocked["id"],
                    }
                },
                allow_errors=True,
            )
            if data.get("errors"):
                message = json.dumps(data["errors"]).lower()
                if "duplicate" in message or "already" in message or "exists" in message:
                    continue
                raise LinearError(
                    f"failed to link {blocker['identifier']} blocks {blocked['identifier']}: "
                    f"{json.dumps(data['errors'], indent=2)}"
                )


def rewrite_issue_bodies(
    client: LinearClient,
    package: Package,
    milestone_map: dict[str, dict[str, Any]],
    issue_map: dict[str, dict[str, Any]],
    project_slug: str,
) -> None:
    for task in package.tasks.values():
        body = issue_body(package, task, issue_map=issue_map)
        update_issue(
            client,
            issue_map[task.id]["id"],
            {
                "projectMilestoneId": milestone_map[task.milestone]["id"],
                "description": body,
                "priority": task.priority,
                "estimate": task.estimate,
            },
        )
    print(f"rewrote issue bodies for project {project_slug}")


def issue_body(package: Package, task: Task, issue_map: dict[str, dict[str, Any]] | None) -> str:
    body = task.body
    if issue_map:
        body = replace_task_refs(body, issue_map)
        blocked_by = render_refs(task.blocked_by, issue_map)
        blocks = render_refs(task.blocks, issue_map)
    else:
        blocked_by = "Resolved during publish."
        blocks = "Resolved during publish."

    return "\n\n".join(
        [
            f"<!-- task-planning-wave: {package.planning_wave} -->",
            f"<!-- task-source-id: {task.id} -->",
            f"<!-- task-source-file: {task.file} -->",
            body,
            "## Linear Dependencies\n\n"
            f"- Blocked by: {blocked_by}\n"
            f"- Blocks: {blocks}",
            "## Linear Metadata\n\n"
            f"- Planning wave: {package.planning_wave}\n"
            f"- Milestone: {task.milestone}\n"
            f"- Priority: {PRIORITY_NAMES.get(task.priority, task.priority)}\n"
            f"- Estimate: {task.estimate}",
            "## Definition of Done\n\n"
            "- All acceptance criteria above are satisfied.\n"
            "- Relevant tests pass, or manual verification evidence is attached.\n"
            "- A PR implementing this issue is merged to the target branch.",
        ]
    )


def replace_task_refs(body: str, issue_map: dict[str, dict[str, Any]]) -> str:
    def replace(match: re.Match[str]) -> str:
        task_id = match.group(0)
        issue = issue_map.get(task_id)
        if not issue:
            return task_id
        return f"[{issue['identifier']}]({issue['url']})"

    return re.sub(r"\b[A-Z][A-Z0-9]+-\d+\b", replace, body)


def render_refs(task_ids: list[str], issue_map: dict[str, dict[str, Any]]) -> str:
    if not task_ids:
        return "None"
    return ", ".join(f"[{issue_map[task_id]['identifier']}]({issue_map[task_id]['url']})" for task_id in task_ids)


def update_overview(client: LinearClient, package: Package, issue_map: dict[str, dict[str, Any]], project: dict[str, Any]) -> None:
    lines = [
        f"# {project['name']} Planning Wave",
        "",
        f"Planning wave: `{package.planning_wave}`",
        "",
        "## Milestones",
        "",
    ]
    for milestone in package.milestones:
        lines.append(f"### {milestone}")
        lines.append("")
        for task in package.tasks.values():
            if task.milestone == milestone:
                issue = issue_map[task.id]
                lines.append(f"- [{issue['identifier']}]({issue['url']}) - {task.title}")
        lines.append("")
    client.call("project_update_content.graphql", {"id": project["id"], "content": "\n".join(lines).strip() + "\n"})
    print("updated project overview")


def write_publish_file(
    path: Path,
    package: Package,
    project_slug: str,
    milestone_map: dict[str, dict[str, Any]],
    issue_map: dict[str, dict[str, Any]],
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "planningWave": package.planning_wave,
        "linearProject": project_slug,
        "publishedAt": datetime.now(timezone.utc).isoformat(),
        "milestones": {
            name: {
                "milestoneId": milestone["id"],
                "name": milestone["name"],
            }
            for name, milestone in milestone_map.items()
        },
        "tasks": {
            task_id: {
                "issue": issue["identifier"],
                "issueId": issue["id"],
                "url": issue["url"],
                "file": package.tasks[task_id].file,
            }
            for task_id, issue in issue_map.items()
        },
    }
    path.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
