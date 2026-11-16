"""Agent guard hook (scripts/agent-guard.sh): what it blocks and what it tells the agent."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUARD = ROOT / "scripts" / "agent-guard.sh"


def run(mode: str, path: str) -> subprocess.CompletedProcess[str]:
    payload = json.dumps({"tool_name": "Write", "tool_input": {"file_path": path}})
    return subprocess.run(
        [str(GUARD), mode],
        input=payload,
        capture_output=True,
        text=True,
        cwd=ROOT,
        check=False,
    )


class PreWriteTests(unittest.TestCase):
    def test_markdown_in_repo_is_blocked_with_reason(self) -> None:
        result = run("pre", str(ROOT / "docs" / "new-note.md"))
        self.assertEqual(result.returncode, 2)
        self.assertIn("agents do not write Markdown files anywhere", result.stderr)
        self.assertIn("put the full text in your reply", result.stderr)

    def test_relative_and_uppercase_markdown_is_blocked(self) -> None:
        self.assertEqual(run("pre", "README.MD").returncode, 2)
        self.assertEqual(run("pre", "AGENTS.md").returncode, 2)

    def test_markdown_outside_repo_is_blocked(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(run("pre", f"{tmp}/notes.md").returncode, 2)

    def test_rule_files_are_blocked_with_reason(self) -> None:
        for rel in [
            "Cargo.toml",
            "clippy.toml",
            "deny.toml",
            "Makefile",
            ".cargo/config.toml",
            "checks/allowed-deps.json",
            ".ast-grep/rules/no-inline-sql.yml",
            "guard/src/main.rs",
            "xtask/src/rls.rs",
            "bins/migrate/queries/catalog_violations.sql",
            "scripts/agent-guard.sh",
            "scripts/tests/test_agent_guard.py",
            ".claude/settings.json",
        ]:
            with self.subTest(rel=rel):
                result = run("pre", str(ROOT / rel))
                self.assertEqual(result.returncode, 2)
                self.assertIn("defines the rules agents work under", result.stderr)

    def test_crate_code_manifests_queries_and_migrations_are_allowed(self) -> None:
        for rel in [
            "crates/db/src/lib.rs",
            "crates/db/Cargo.toml",
            "crates/db/queries/catalog/list_items.sql",
            "crates/db/migrations/0002_new.sql",
            "bins/api-server/src/main.rs",
        ]:
            with self.subTest(rel=rel):
                self.assertEqual(run("pre", str(ROOT / rel)).returncode, 0)


class PostWriteTests(unittest.TestCase):
    def write_rs(self, source: str) -> Path:
        directory = tempfile.TemporaryDirectory(dir=ROOT / "crates")
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "sample.rs"
        path.write_text(source)
        return path

    def test_rust_comment_is_rejected_with_location_and_reason(self) -> None:
        path = self.write_rs("fn a() {}\n// why\nfn b() {}\n")
        result = run("post", str(path))
        self.assertEqual(result.returncode, 2)
        self.assertIn(f"{path}:2: comment", result.stderr)
        self.assertIn("implement the clean solution", result.stderr)

    def test_rust_doc_comment_is_rejected(self) -> None:
        result = run("post", str(self.write_rs("/// docs\npub fn a() {}\n")))
        self.assertEqual(result.returncode, 2)
        self.assertIn("doc comment", result.stderr)

    def test_rust_lint_suppression_is_rejected(self) -> None:
        source = '#[expect(clippy::unwrap_used, reason = "fine for now")]\nfn a() {}\n'
        result = run("post", str(self.write_rs(source)))
        self.assertEqual(result.returncode, 2)
        self.assertIn("lint suppression", result.stderr)
        self.assertIn("Fix the code the lint points at", result.stderr)

    def test_check_fixtures_may_contain_suppressions(self) -> None:
        fixture = ROOT / "checks/lints/fail/src/allow_attributes.rs"
        self.assertEqual(run("post", str(fixture)).returncode, 0)

    def test_clean_rust_passes(self) -> None:
        self.assertEqual(
            run(
                "post", str(self.write_rs('fn a() -> &\'static str { "//" }\n'))
            ).returncode,
            0,
        )


class OtherCheckoutTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.checkout = Path(tmp.name) / "wt"
        subprocess.run(
            [
                "git",
                "-C",
                str(ROOT),
                "worktree",
                "add",
                "--detach",
                "-q",
                str(self.checkout),
            ],
            check=True,
        )
        self.addCleanup(
            subprocess.run,
            [
                "git",
                "-C",
                str(ROOT),
                "worktree",
                "remove",
                "--force",
                str(self.checkout),
            ],
            check=False,
        )

    def test_rust_comment_in_another_worktree_is_rejected(self) -> None:
        path = self.checkout / "guard" / "src" / "probe.rs"
        path.write_text("// why\nfn a() {}\n")
        self.assertEqual(run("post", str(path)).returncode, 2)

    def test_rule_file_in_another_worktree_is_blocked(self) -> None:
        self.assertEqual(run("pre", str(self.checkout / "deny.toml")).returncode, 2)

    def test_rust_file_in_unrelated_repo_is_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run(["git", "init", "-q", tmp], check=True)
            path = Path(tmp) / "a.rs"
            path.write_text("// why\nfn a() {}\n")
            self.assertEqual(run("post", str(path)).returncode, 0)


if __name__ == "__main__":
    unittest.main()
