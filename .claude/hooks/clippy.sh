#!/usr/bin/env bash
# Stop: run clippy at the end of a turn and hand any warnings back to Claude (exit 2).
# stop_hook_active is true when Claude is already continuing because of this hook, so
# it runs at most once per turn and cannot loop. No-op until the project has a Cargo.toml.
input=$(cat)
[ "$(jq -r '.stop_hook_active // false' <<<"$input")" = true ] && exit 0
manifest="${CLAUDE_PROJECT_DIR:-.}/Cargo.toml"
[ -f "$manifest" ] || exit 0
if ! out=$(cargo clippy --manifest-path "$manifest" --all-targets --quiet -- -D warnings 2>&1); then
  printf 'cargo clippy found problems:\n%s\n' "$out" >&2
  exit 2
fi
