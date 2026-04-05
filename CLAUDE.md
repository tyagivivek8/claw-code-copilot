# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Claw Code is an autonomous CLI agent harness (binary: `claw`) that connects to the Anthropic API, executes tools (bash, file ops, MCP, LSP, plugins), manages sessions, and enforces permissions. The project is autonomously maintained by coding agents ("claws"), not a conventional dev team. See `PHILOSOPHY.md` for the coordination model (OmX, clawhip, OmO).

## Build & verify (all from `rust/`)

```bash
cargo build --workspace              # build all crates
cargo fmt --all --check              # format check (CI runs this)
cargo clippy --workspace --all-targets -- -D warnings   # lint
cargo test --workspace               # all tests
cargo test -p rusty-claude-cli       # CLI crate tests only
cargo test -p rusty-claude-cli --test mock_parity_harness  # parity harness (10 scenarios)
```

Python porting workspace (secondary):
```bash
python3 -m src.main summary          # porting summary
python3 -m unittest discover -s tests -v   # Python tests
```

## Workspace lints

Configured in `rust/Cargo.toml`: `unsafe_code` is **forbidden**, clippy `all` + `pedantic` are warnings. All clippy warnings are CI-blocking (`-D warnings`).

## Repository layout

| Path | Purpose |
|------|---------|
| `rust/` | **Active Rust workspace** — 9 crates, the real implementation |
| `rust/crates/rusty-claude-cli/` | Main CLI binary (`claw`), REPL, one-shot, streaming |
| `rust/crates/runtime/` | Core agentic loop, config, session, permissions, MCP, file ops, bash validation |
| `rust/crates/api/` | Anthropic HTTP client, SSE streaming, auth |
| `rust/crates/tools/` | Tool dispatch — 40 tool specs (bash, read, write, edit, glob, grep, web, agent, MCP, LSP, etc.) |
| `rust/crates/commands/` | Slash command registry (67+ commands) |
| `rust/crates/plugins/` | Plugin system and hook lifecycle |
| `rust/crates/telemetry/` | Session tracing and cost tracking |
| `rust/crates/mock-anthropic-service/` | Deterministic mock for parity testing (10 scenarios, 19 captured requests) |
| `rust/crates/compat-harness/` | TypeScript manifest extraction (legacy porting) |
| `src/` | Python porting workspace — mirrors command/tool inventories for analysis, not a runtime replacement |
| `tests/` | Python workspace verification |

## Architecture (Rust)

**Request flow:** CLI (`main.rs`) -> `ConversationRuntime` (agentic turn loop in `runtime/conversation.rs`) -> `AnthropicClient` (SSE streaming in `api/`) -> tool dispatch (`tools/lib.rs`) -> permission enforcement (`runtime/permissions.rs`) -> tool execution -> loop or finish.

Key runtime modules:
- `conversation.rs` — agentic turn loop: prompt assembly, API call, tool dispatch, result collection
- `config.rs` — config hierarchy loader (`.claw.json`, settings, local overrides)
- `session.rs` — JSONL persistence and session resumption
- `permissions.rs` — policy engine (read-only, workspace-write, danger-full-access)
- `bash_validation.rs` — command analysis (destructive detection, read-only gating, 9+ submodules)
- `mcp_stdio.rs` — MCP JSON-RPC protocol, server lifecycle with degraded-mode reporting
- `hooks.rs` — hook runner infrastructure
- `file_ops.rs` — read/write/edit/glob/grep with boundary checks and binary detection

Model aliases: `opus` -> `claude-opus-4-6`, `sonnet` -> `claude-sonnet-4-6`, `haiku` -> `claude-haiku-4-5`

## CI

GitHub Actions (`rust-ci.yml`): runs `cargo fmt` and `cargo test -p rusty-claude-cli` on push/PR to `main`. Triggers on `rust/**` path changes. Branch patterns: `main`, `gaebal/**`, `omx-issue-*`.

## Working agreement

- Prefer small, reviewable changes. Keep `src/` and `tests/` consistent when behavior changes.
- Keep shared defaults in `.claude.json`; reserve `.claude/settings.local.json` for machine-local overrides.
- Do not overwrite this `CLAUDE.md` automatically; update it intentionally when repo workflows change.
- Parity tracking lives in `PARITY.md` — update it when tool surface or mock scenarios change.
