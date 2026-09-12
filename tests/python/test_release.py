from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import cast

import pytest

REPOSITORY_ROOT = Path(__file__).parents[2]
pytestmark = pytest.mark.skipif(os.name == "nt", reason="Unix release helper")

# Build tools and GitHub are stubbed; Git uses a real, local bare remote.
TOOL_STUB = """\
import json
import os
import re
import sys
from pathlib import Path

name = Path(sys.argv[0]).name
args = sys.argv[1:]
with Path(os.environ["RELEASE_TEST_LOG"]).open("a") as log:
    log.write(json.dumps([name, *args]) + "\\n")
if name == "cargo" and args[0] == "check":
    manifest = Path("Cargo.toml").read_text()
    version = re.search(r'^version = "([^"]+)"', manifest, re.M)[1]
    lock = Path("fuzz/Cargo.lock" if "--manifest-path" in args else "Cargo.lock")
    updated = re.sub(r'version = "[^"]+"', f'version = "{version}"', lock.read_text())
    lock.write_text(updated)
if os.environ.get("RELEASE_TEST_FAIL") == name:
    sys.exit(1)
if name == "gh" and args[:2] == ["release", "view"]:
    sys.exit(0 if os.environ.get("RELEASE_TEST_EXISTS") else 1)
if name == "uv" and "--out" in args:
    output = Path(args[args.index("--out") + 1])
    (output / ("package.whl" if "build" in args else "package.tar.gz")).touch()
"""


@dataclass
class ReleaseRepo:
    root: Path
    remote: Path
    log: Path
    env: dict[str, str]

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", *args],
            cwd=self.root,
            env=self.env,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def run(self, *args: str, answer: str = "y\n") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", "dev-bin/release.sh", *args],
            cwd=self.root,
            env=self.env,
            input=answer,
            capture_output=True,
            text=True,
            timeout=30,
        )

    @property
    def calls(self) -> list[list[str]]:
        if not self.log.exists():
            return []
        return [
            cast(list[str], json.loads(line))
            for line in self.log.read_text().splitlines()
        ]

    def changelog(self, version: str = "1.2.3", day: str | None = None) -> None:
        (self.root / "CHANGELOG.md").write_text(
            "# Changelog\n\n## [Unreleased]\n\n"
            f"## [{version}] - {day or date.today().isoformat()}\n\n"
            "### Changed\n\n- Release notes.\n\n"
            "## [1.2.2] - 2000-01-01\n\n- Previous release.\n"
        )

    def commit(self, message: str) -> None:
        self.git("add", ".")
        self.git("commit", "-m", message)

    def assert_unpublished(self, original_head: str) -> None:
        assert self.git("rev-parse", "HEAD") == original_head
        assert self.git("ls-remote", "origin", "refs/heads/publish-next") == ""
        assert not any(call[:3] == ["gh", "release", "create"] for call in self.calls)
        assert all(
            "--dry-run" in call
            for call in self.calls
            if call[:2] == ["cargo", "publish"]
        )


@pytest.fixture
def release_repo(tmp_path: Path) -> ReleaseRepo:
    root = tmp_path / "repo"
    root.mkdir()
    tool_dir = tmp_path / "bin"
    tool_dir.mkdir()
    artifact_dir = tmp_path / "artifacts"
    artifact_dir.mkdir()
    env = {
        key: value for key, value in os.environ.items() if not key.startswith("GIT_")
    }
    env.update(
        PATH=f"{tool_dir}{os.pathsep}{os.environ['PATH']}",
        GIT_CONFIG_GLOBAL=str(tmp_path / "gitconfig"),
        GIT_CONFIG_NOSYSTEM="1",
        GIT_TERMINAL_PROMPT="0",
        RELEASE_TEST_LOG=str(tmp_path / "calls.jsonl"),
        TMPDIR=str(artifact_dir),
    )
    repo = ReleaseRepo(root, tmp_path / "origin.git", tmp_path / "calls.jsonl", env)
    for tool in ("cargo", "uv", "gh", "scripts/check"):
        path = root / tool if "/" in tool else tool_dir / tool
        path.parent.mkdir(exist_ok=True)
        path.write_text(f"#!{sys.executable}\n{TOOL_STUB}")
        path.chmod(0o755)
    (root / "dev-bin").mkdir()
    shutil.copy2(REPOSITORY_ROOT / "dev-bin/release.sh", root / "dev-bin/release.sh")
    (root / "fuzz").mkdir()
    for name in ("Cargo.toml", "Cargo.lock", "fuzz/Cargo.lock"):
        (root / name).write_text(
            '[package]\nname = "maxminddb-polars"\nversion = "1.2.2"\n'
        )
    repo.changelog()
    repo.git("init", "--initial-branch=main")
    repo.git("config", "user.name", "Release Test")
    repo.git("config", "user.email", "release@example.invalid")
    repo.commit("Initial source and committed changelog")
    repo.git("init", "--bare", "--initial-branch=main", str(repo.remote))
    repo.git("remote", "add", "origin", str(repo.remote))
    repo.git("push", "--set-upstream", "origin", "main")
    repo.git("switch", "-c", "publish-next")
    return repo


@pytest.mark.parametrize(
    ("version", "pep440"),
    [
        ("1.2.3", "1.2.3"),
        ("1.2.3-alpha.1", "1.2.3a1"),
        ("1.2.3-beta.2", "1.2.3b2"),
        ("1.2.3-rc.3", "1.2.3rc3"),
    ],
)
def test_release_commits_pushes_and_creates_release(
    release_repo: ReleaseRepo, version: str, pep440: str
) -> None:
    repo = release_repo
    if version != "1.2.3":
        repo.changelog(version)
        repo.commit("Set release version")
    head = repo.git("rev-parse", "HEAD")

    result = repo.run()

    assert result.returncode == 0, result.stdout + result.stderr
    released_head = repo.git("rev-parse", "HEAD")
    assert released_head != head
    assert repo.git("log", "-1", "--format=%s") == f"Prepare v{version} release"
    assert repo.git("status", "--porcelain") == ""
    assert (
        repo.git("ls-remote", "origin", "refs/heads/publish-next").split()[0]
        == released_head
    )
    assert repo.git("rev-parse", "@{upstream}") == released_head
    for name in ("Cargo.toml", "Cargo.lock", "fuzz/Cargo.lock"):
        assert f'version = "{version}"' in (repo.root / name).read_text()
    calls = repo.calls
    assert ["check"] in calls
    assert ["cargo", "publish", "--dry-run", "--locked", "--allow-dirty"] in calls
    inspection = next(call for call in calls if "scripts/inspect_artifacts.py" in call)
    assert inspection[inspection.index("--expected-version") + 1] == pep440
    assert any("twine" in call and "--strict" in call for call in calls)
    assert calls[-1] == [
        "gh",
        "release",
        "create",
        "--target",
        released_head,
        "--title",
        version,
        "--notes",
        "\n### Changed\n\n- Release notes.",
        f"v{version}",
    ]


def test_release_with_versions_already_updated(release_repo: ReleaseRepo) -> None:
    repo = release_repo
    for name in ("Cargo.toml", "Cargo.lock", "fuzz/Cargo.lock"):
        path = repo.root / name
        path.write_text(path.read_text().replace("1.2.2", "1.2.3"))
    repo.commit("Update versions")
    head = repo.git("rev-parse", "HEAD")

    result = repo.run()

    assert result.returncode == 0, result.stdout + result.stderr
    assert repo.git("rev-parse", "HEAD") == head
    assert repo.git("ls-remote", "origin", "refs/heads/publish-next").split()[0] == head
    assert repo.calls[-1][:5] == ["gh", "release", "create", "--target", head]


@pytest.mark.parametrize("answer", ["n\n", "\n", ""])
def test_declining_restores_versions(release_repo: ReleaseRepo, answer: str) -> None:
    repo = release_repo
    head = repo.git("rev-parse", "HEAD")

    result = repo.run(answer=answer)

    assert result.returncode != 0
    assert "Validated v1.2.3" in result.stdout
    assert repo.git("status", "--porcelain") == ""
    repo.assert_unpublished(head)


@pytest.mark.parametrize("tool", ["cargo", "check", "uv"])
def test_validation_failure_restores_versions(
    release_repo: ReleaseRepo, tool: str
) -> None:
    repo = release_repo
    head = repo.git("rev-parse", "HEAD")
    repo.env["RELEASE_TEST_FAIL"] = tool

    result = repo.run()

    assert result.returncode != 0
    assert repo.git("status", "--porcelain") == ""
    repo.assert_unpublished(head)


def test_dry_run_does_not_change_or_publish_source(release_repo: ReleaseRepo) -> None:
    repo = release_repo
    repo.changelog("1.2.2", "2000-01-01")
    repo.commit("Record previous release")
    repo.git("switch", "-C", "main")
    repo.git("remote", "remove", "origin")
    head = repo.git("rev-parse", "HEAD")

    result = repo.run("--dry-run", answer="")

    assert result.returncode == 0, result.stdout + result.stderr
    assert "Dry run complete" in result.stdout
    assert repo.git("rev-parse", "HEAD") == head
    assert repo.git("status", "--porcelain") == ""
    assert ["check"] in repo.calls
    assert not any(call[0] == "gh" for call in repo.calls)
    assert ["cargo", "publish", "--dry-run", "--locked", "--allow-dirty"] in repo.calls


@pytest.mark.parametrize(
    "state", ["main", "detached", "dirty", "staged", "untracked", "old_date"]
)
def test_invalid_source_is_rejected(release_repo: ReleaseRepo, state: str) -> None:
    repo = release_repo
    if state == "main":
        repo.git("switch", "main")
    elif state == "detached":
        repo.git("switch", "--detach")
    elif state in ("dirty", "staged"):
        repo.changelog("1.2.4")
        if state == "staged":
            repo.git("add", "CHANGELOG.md")
    elif state == "untracked":
        (repo.root / "untracked").touch()
    else:
        repo.changelog(day="2000-01-01")
        repo.commit("Set old date")
    head = repo.git("rev-parse", "HEAD")
    status = repo.git("status", "--porcelain")

    result = repo.run()

    assert result.returncode != 0
    assert repo.calls == []
    assert repo.git("status", "--porcelain") == status
    repo.assert_unpublished(head)


def test_branch_behind_main_is_rejected(release_repo: ReleaseRepo) -> None:
    repo = release_repo
    repo.git("switch", "main")
    (repo.root / "new-file").touch()
    repo.commit("Advance main")
    repo.git("push", "origin", "main")
    repo.git("switch", "publish-next")
    head = repo.git("rev-parse", "HEAD")

    result = repo.run()

    assert result.returncode != 0
    assert "behind origin/main" in result.stderr
    assert repo.git("status", "--porcelain") == ""
    repo.assert_unpublished(head)


@pytest.mark.parametrize("existing", ["local_tag", "remote_tag", "release"])
def test_existing_tag_or_release_is_rejected(
    release_repo: ReleaseRepo, existing: str
) -> None:
    repo = release_repo
    if existing == "release":
        repo.env["RELEASE_TEST_EXISTS"] = "1"
    else:
        repo.git("tag", "v1.2.3")
        if existing == "remote_tag":
            repo.git("push", "origin", "v1.2.3")
            repo.git("tag", "-d", "v1.2.3")
    head = repo.git("rev-parse", "HEAD")

    result = repo.run()

    assert result.returncode != 0
    assert "already exists" in result.stderr
    assert repo.git("status", "--porcelain") == ""
    repo.assert_unpublished(head)


def test_commit_failure_restores_versions(release_repo: ReleaseRepo) -> None:
    repo = release_repo
    hook = repo.root / ".git/hooks/pre-commit"
    hook.write_text("#!/bin/sh\nexit 1\n")
    hook.chmod(0o755)
    head = repo.git("rev-parse", "HEAD")

    result = repo.run()

    assert result.returncode != 0
    assert repo.git("status", "--porcelain") == ""
    repo.assert_unpublished(head)


def test_push_failure_does_not_create_release(release_repo: ReleaseRepo) -> None:
    repo = release_repo
    hook = repo.remote / "hooks/pre-receive"
    hook.write_text("#!/bin/sh\nexit 1\n")
    hook.chmod(0o755)
    head = repo.git("rev-parse", "HEAD")

    result = repo.run()

    assert result.returncode != 0
    assert repo.git("rev-parse", "HEAD") != head
    assert repo.git("status", "--porcelain") == ""
    assert repo.git("ls-remote", "origin", "refs/heads/publish-next") == ""
    assert not any(call[:3] == ["gh", "release", "create"] for call in repo.calls)


@pytest.mark.parametrize(
    "args", [("--publish",), ("--unknown",), ("--dry-run", "extra")]
)
def test_invalid_arguments_are_rejected(
    release_repo: ReleaseRepo, args: tuple[str, ...]
) -> None:
    repo = release_repo
    head = repo.git("rev-parse", "HEAD")

    result = repo.run(*args)

    assert result.returncode == 2
    assert "Usage: dev-bin/release.sh [--dry-run]" in result.stderr
    assert repo.calls == []
    repo.assert_unpublished(head)
