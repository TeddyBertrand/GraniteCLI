# CLAUDE.md

Guidance for Claude Code working in this repo. See `docs/PROJECT.md` for full project goals/architecture.

## What this is

GraniteCLI — Rust CLI agent (ReAct loop) driving free/open LLM providers (Groq, Gemini, Ollama). Cargo workspace, 4 crates.

## Workspace layout

- `crates/cli` (pkg `granite-cli`, bin `granite`) — clap + cliclack, entry point / UI only.
- `crates/core` (pkg `granite-core`) — ReAct agent loop, conversation context/history.
- `crates/provider` (pkg `granite-provider`) — `trait LlmProvider`, per-vendor HTTP impls (Groq, Gemini, Ollama).
- `crates/tools` (pkg `granite-tools`) — `trait Tool`, local fs/bash tool impls.

Dependency graph: `cli` → `core` → `provider` + `tools`. Providers/tools crates must not depend on `cli` or `core`.

Package names are `granite-*` but dependency keys stay `core`/`provider`/`tools` (aliased via `package =` in root `Cargo.toml`) — so `use core::...` etc still works in code.

Exception: `cli` uses `granite_core = { package = "granite-core", ... }` instead of `core.workspace = true` — a dep literally named `core` shadows Rust's sysroot `core` crate in the extern prelude, which breaks any derive macro expanding to unqualified `core::...` (hit this with `clap::Parser`). Use `granite_core::...` in `cli` crate code.

## Conventions

- **Errors:** `thiserror` typed enums in lib crates (`core`, `provider`, `tools`) — keep them matchable (agent needs to branch on e.g. rate-limit vs auth). `anyhow` only at the `cli` boundary (`main() -> anyhow::Result<()>`).
- **Deps:** pin versions once in root `[workspace.dependencies]`, crates inherit via `dep.workspace = true`. Don't add a dep version directly in a crate's own `Cargo.toml`.
- **Async:** `tokio` full runtime, `async-trait` for trait methods on `LlmProvider`/`Tool`.
- Currently skeleton stage — most `.rs` files are stubs/empty. Don't assume implementations exist; check before referencing.
- **Tests:** put `#[cfg(test)]` unit tests in a sibling `<module>_test.rs` file, wired via `#[path = "<module>_test.rs"] mod tests;` — not inline in the module file. See `crates/provider/src/groq.rs` / `groq_test.rs`.

## Commit rules

- Format: `type(scope): description` (e.g. `feat(cli): add arg parsing`) — one line only, no body.
- Types: feat/fix/refacto/docs/chore.
- No co-author trailer.
- One feature/change per commit — don't mix unrelated changes. Never bundle by file-count, always by feature (2 features touching 2 files each = 2 commits, not 1).
- 3-4 files is a max cap, not a target: if one feature spans more files than that, split it into multiple commits.

## PR rules

- No "Generated with Claude Code" / co-author / attribution footer in PR title or body. PR content only.

## Docs

- `docs/PROJECT.md` — goals, stack rationale, architecture, status/next-steps. Keep it in sync with real decisions (not brainstorm.md, which is the original French draft, kept as-is for history).
