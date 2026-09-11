# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

The MCP sibling of `~/gemma-rust` (which calls the model's HTTP endpoint directly). This CLI
launches a Gemma 4 rig's own MCP server — `python3 server.py` in the rig directory, over stdio —
using the `rmcp` 3.3 client, and asks the question through the rig's tools. **It is a demo**: it
prints every step of the MCP exchange (see "Demo output").

Two rigs, chosen with `--rig`:

- `local` → `~/gemma4-dev/local-llamacpp-1650ti-2b-q4_0` (llama.cpp on the local GPU)
- `cloudrun` → `~/gemma4-dev/gpu-2B-cloudrun-devops-agent` (vLLM on Cloud Run)

Each rig's `CLAUDE.md` is the reference for its server. Nothing is imported from the rigs; they
are only launched.

## Only read-only tools — this is a hard rule

The rig servers also expose tools that deploy, destroy, scale, start, or stop things
(`cloudrun_deploy`, `cloudrun_destroy`, `cloudrun_update_scaling`, `cloudrun_save_hf_token`,
`start_model_server`, `stop_model_server`, …). They are billed or destructive. The CLI calls only
the tools named in `Rig::status_tools` and `Rig::query_tool`; never add one of those, and never
add a generic "call any tool" option. The same applies to interactive-mode commands: `/status` only
runs `Rig::status_tools`.

## Gotchas

- **The server runs with the rig directory as its working directory** — the local rig opens
  `server.py` and `tpu.env` by relative path.
- The rigs need `mcp>=2` in the **system `python3`** (2.2.0 installed). If a dependency is
  missing, `pip install` it into system python3 — the rigs forbid virtualenvs.
- **Failure is reported in the text, not the protocol**: rig tools return markdown starting with
  `❌` and leave `isError` false. `call()` treats a leading `❌` as failure; keep that. For the
  query tool a leading `📡` means "reasoning only, no answer yet" (Gemma 4 ran out of
  `max_tokens` mid-thought) — `ask()` reports it as no answer. On status tools `📡` is just
  informational.
- The query tools differ: local `query_model(prompt, max_tokens)` suppresses the reasoning and
  reports token counts; Cloud Run `cloudrun_query_gemma4_with_stats(prompt)` takes no
  `max_tokens`, reports TTFT and tok/s, and its token count is approximate (it counts stream
  chunks).
- **No project id or service URL belongs in this repo.** The Cloud Run server resolves them itself
  (its defaults, or `gcloud`). Its status tools print the service URL — replace it with a
  placeholder in any published sample output.
- `rmcp`: `CallToolRequestParams` is `#[non_exhaustive]`, so build it with
  `CallToolRequestParams::new(name).with_arguments(map)`.

## Demo output

Every run prints, in labelled sections: the server command, working dir and pid; the `initialize`
latency, server name/version, protocol version and capabilities; every tool the server offers
(`*` marks the ones called; `--schemas` adds input schemas); each `tools/call` with its arguments,
latency, `isError`, and the returned text (a `structuredContent` that only mirrors the text is
not printed twice); and the server's stderr log. Exit codes: 0 answered, 1 error, 2 the query
tool failed or returned no answer.

`-i/--interactive` (`make chat`, `make chat-cloud`) keeps one MCP session open. The startup steps
print once, then each prompt goes through the same `ask()` as a single run, followed by the new
stderr lines (`print_log` tracks how many it has shown). The tools take one prompt, so there is no
conversation memory. `readline` runs inside `block_in_place` so the rmcp transport keeps running.
A turn's error is printed and the loop continues; exit 0 on `/quit` or Ctrl-D.

`--status` (`make status`, `make status-cloud`) runs only `Rig::status_tools` and skips the query;
exit 2 if one reports failure. `make help` lists
the other targets (`debug`, `prod`, `lint`, `test`, `ci`, `run`, `clean`, …).
