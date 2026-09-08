# GraniteCLI — Project Documentation

**GraniteCLI** is a modern, modular, agentic command-line tool built to drive generative AI models that are public, free, or openly accessible (Groq, Google Gemini, local models via Ollama). Inspired by the ergonomics of tools like Claude Code, it lets a software agent interact directly with a local dev environment — read/write files, run shell commands — fluidly and safely.

---

## 1. Goals

- **Agentic Loop (ReAct):** Let an LLM analyze a problem, decide on actions via tool calls, execute them locally, and iterate until the task is solved.
- **Provider freedom:** Not locked into one paid ecosystem or vendor — a unified abstraction layer lets you swap between LLMs (Groq, Gemini, Ollama, etc.) instantly.
- **Max terminal ergonomics:** Clean, interactive text UI (intros, spinners, selectors) without the weight of a full-screen TUI.
- **Modularity & maintainability:** Rigorous Rust workspace structure for strong performance, a single lightweight binary, and strict separation of concerns.

---

## 2. Tech Stack

Crate choices driven by performance, safety, and ecosystem maturity:

- **`tokio`** — async runtime, required for non-blocking network I/O and the agent's event loop.
- **`reqwest`** — HTTP client (`json` + `stream` features) for talking to LLM APIs and consuming streamed responses (SSE).
- **`serde` / `serde_json`** — serialization for chat messages and dynamic parsing of tool-call JSON payloads.
- **`clap`** — CLI argument parser (`derive` attr) for options and subcommands.
- **`cliclack`** — minimal, elegant interactive prompts (inspired by `@clack/prompts`) for intros, status messages, user confirmations.
- **`async-trait`** — async methods in traits, needed for provider and tool contracts.
- **`ignore`** — from the ripgrep ecosystem, walks a project's file tree while respecting `.gitignore`.
- **`thiserror`** — typed error enums in lib crates (`provider`, `tools`, `core`), so callers can match on variant (e.g. rate-limit vs auth vs parse failure) and the agent loop can branch/retry accordingly.
- **`anyhow`** — used only at the `cli` boundary; `main() -> anyhow::Result<()>` wraps whatever bubbles up from lower crates and adds `.context()` for user-facing error messages.

---

## 3. Workspace Architecture

Cargo workspace under `crates/`, one crate per responsibility.

### `crates/cli`
Entry point / UI layer.
- Parse CLI args via `clap`.
- Drive visual output (intros, spinners, messages) via `cliclack`.
- Init config, kick off the agent engine in `core`.

### `crates/core`
Brains of the app.
- Implement the agent state machine (ReAct: Reasoning and Acting).
- Manage conversation history, message context, token tracking.
- Coordinate LLM requests and tool execution.

### `crates/provider`
Network / AI abstraction layer.
- Expose a unified contract (`trait LlmProvider`), provider-agnostic.
- Encapsulate per-model HTTP calls (Groq, Gemini, Ollama, etc.).
- Parse response streams into a normalized request/response structure.

### `crates/tools`
Agent's hands — local system actions.
- Define a generic `trait Tool` contract.
- File tools (`fs.rs`), filtering irrelevant dirs via `ignore`.
- Shell exec tool (`bash.rs`) with validation/safety mechanisms.

---

## 4. Current Status

Workspace skeleton only (`squelette.sh` scaffolded empty files) — no implementation yet. `Cargo.toml` workspace manifest and crate manifests present but not filled in.

## 5. Next Steps

- Fill in workspace `Cargo.toml` (members, shared deps/versions).
- Define `trait LlmProvider` in `provider/src/traits.rs`.
- Define `trait Tool` in `tools/src/traits.rs`.
- Implement Groq provider first (simplest free API) as reference impl.
- Build minimal ReAct loop in `core/src/agent.rs` against one provider + `fs` tools.
- Wire `cli` entrypoint with `clap` + `cliclack` intro/spinner.
