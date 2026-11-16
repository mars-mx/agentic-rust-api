#!/usr/bin/env bash
# Claude Code hook: `pre` runs before Write/Edit/MultiEdit/NotebookEdit, `post` after.
# Exit 2 blocks the call (pre) or hands the finding back to the agent (post); stderr is the message.
set -euo pipefail

mode="${1:?usage: agent-guard.sh pre|post}"
root="$(cd "$(dirname "$0")/.." && pwd -P)"

reject() {
  printf '%s\n' "$@" >&2
  exit 2
}

command -v jq >/dev/null || reject "agent-guard: jq is missing, so this hook cannot check the change. Install jq."

path="$(jq -r '.tool_input.file_path // .tool_input.notebook_path // empty')"
[ -n "$path" ] || exit 0
case "$path" in /*) ;; *) path="$PWD/$path" ;; esac
dir="$(dirname "$path")"
while [ ! -d "$dir" ]; do dir="$(dirname "$dir")"; done
path="$(cd "$dir" && pwd -P)${path#"$dir"}"

if [ "$mode" = pre ]; then
  case "$path" in
    *.[mM][dD])
      reject \
        "Blocked: agents do not write Markdown files anywhere ($path)." \
        "That covers this repo, other checkouts and worktrees of it, planning folders, the scratchpad, and any path outside the repo." \
        "Agents use Markdown to write down why not doing the right thing was fine, or to turn one small correction into a rule that can never be broken again. Nothing enforces such a doc, it goes stale, and the next agent follows it as the established pattern." \
        "A lesson belongs in a check: a lint rule, a type, a test. If you were about to document a workaround, a shortcut or debt, implement the clean solution under the rules of this repo instead." \
        "If the user asks you to write something down, put the full text in your reply. The user decides whether it becomes a file and where. Do not suggest a command that writes the file for you either." \
        "Writing the file through Bash, python, sed or any other program breaks the same rule. It is a violation, not a workaround."
      ;;
  esac
fi

checkout="$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null)" || exit 0
checkout="$(cd "$checkout" && pwd -P)"
[ "$(cd "$(git -C "$checkout" rev-parse --git-common-dir)" && pwd -P)" = "$(cd "$(git -C "$root" rev-parse --git-common-dir)" && pwd -P)" ] || exit 0
case "$path" in "$checkout"/*) ;; *) exit 0 ;; esac
rel="${path#"$checkout"/}"

if [ "$mode" = pre ]; then
  case "$rel" in
    Cargo.toml | clippy.toml | deny.toml | sgconfig.yml | rust-toolchain.toml | Makefile | .cargo/* | \
    .ast-grep/* | checks/* | guard/* | xtask/* | bins/migrate/* | \
    scripts/agent-guard.sh | scripts/tests/* | .claude/settings.json | .claude/settings.local.json)
      reject \
        "Blocked: $rel defines the rules agents work under, so agents do not change it." \
        "That covers the workspace lints, clippy, cargo-deny, ast-grep rules, the dependency allowlist, the check fixtures, the migration and catalog checks, the toolchain, the Makefile, this hook and the settings that load it." \
        "If a check rejects your code, fix the code. If you think the rule is wrong, or a new crate or dependency needs an allowlist entry, stop and tell the user what to change and why. The user edits these files." \
        "Changing them through Bash, python, sed, git or any other program breaks the same rule. It is a violation, not a workaround."
      ;;
  esac
  exit 0
fi

case "$rel" in
  checks/*) ;;
  *.rs)
    [ -f "$path" ] || exit 0
    status=0
    found="$(cargo run --quiet --release --locked --manifest-path "$root/guard/Cargo.toml" -- "$path" 2>&1)" || status=$?
    case "$status" in
      0) ;;
      1) reject \
        "Rejected: Rust code in this repo has no comments and no lint suppressions. That includes doc comments, #[doc], #[allow], #[expect] and #[warn], also inside #[cfg_attr]." \
        "$found" \
        "Comments are mostly used to justify workarounds, bad decisions, tech debt or unclean implementations. They go stale, nothing enforces them, and the next agent copies the code they excuse as the established pattern." \
        "If the comment explains why a shortcut is fine, the shortcut is the problem. Drop it and implement the clean solution under the rules of this repo." \
        "If the code needs a comment to be understood, make the code say it. Rename, extract a function, or encode the rule in a type." \
        "A lint suppression is the same excuse in another place, with the reason string as the comment. Fix the code the lint points at. If you think the lint is wrong for this code, stop and tell the user; lint configuration changes centrally, never inline." \
        "If the clean solution is out of reach, stop and tell the user instead of writing the excuse down." \
        "Remove every comment in this file, including ones that were there before your edit. Old comments get copied just the same." ;;
      *) reject "agent-guard: the comment checker could not run (exit $status):" "$found" ;;
    esac
    ;;
esac
