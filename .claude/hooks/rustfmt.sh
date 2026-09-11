#!/usr/bin/env bash
# PostToolUse Write|Edit: format the crate after a .rs file changes.
# No-op until the project has a Cargo.toml.
f=$(jq -r '.tool_response.filePath // .tool_input.file_path // empty')
case "$f" in *.rs) ;; *) exit 0 ;; esac
manifest="${CLAUDE_PROJECT_DIR:-.}/Cargo.toml"
[ -f "$manifest" ] || exit 0
cargo fmt --manifest-path "$manifest"
