# GraniteCLI — Issue Plan

Working list of GitHub issues to open, in build order. Skeleton stage: workspace/Cargo.toml wired, all `.rs` files empty stubs. Order follows the dependency graph `cli → core → provider + tools` — providers/tools have no upstream deps so they unblock in parallel, `core` needs both, `cli` needs `core`.

Legend: **P0** blocking/critical path, **P1** needed for MVP, **P2** polish/later. Time = rough solo-dev estimate, not calendar time.

---

## 1. `trait Tool` contract (`tools/src/traits.rs`)
**Tags:** `tools`, `design`, `P0` · **Time:** 1-2h

Define the generic `Tool` trait: name/description for LLM tool-calling schema, async `execute(args: serde_json::Value) -> Result<ToolOutput, ToolError>`. Needs `thiserror` `ToolError` enum (not-found, invalid-args, io, denied). Blocks `fs.rs`, `bash.rs`, and `core`'s dispatcher.

## 2. `trait LlmProvider` contract (`provider/src/traits.rs`)
**Tags:** `provider`, `design`, `P0` · **Time:** 1-2h

Define unified request/response types (chat message, role, tool-call schema, streamed chunk) and the async `LlmProvider` trait (`chat(...) -> Result<Response, ProviderError>` + streaming variant). `thiserror` `ProviderError` enum: matchable variants for rate-limit, auth, parse-failure, network — `core` branches/retries on these. Blocks Groq/Gemini impls and `core`.

**Plan (in progress):**
- `Role` enum: `System`, `User`, `Assistant`, `Tool`.
- `ToolCall { id: String, name: String, arguments: serde_json::Value }` — vendor-agnostic call shape.
- `ToolDefinition { name: String, description: String, parameters: serde_json::Value }` — JSON-schema params, passed by `core` to advertise tools.
- `Message { role: Role, content: Option<String>, tool_calls: Vec<ToolCall>, tool_call_id: Option<String> }` — one struct covers user/assistant/tool turns; `tool_call_id` set only on `Role::Tool` replies.
- `ChatRequest { messages: Vec<Message>, tools: Vec<ToolDefinition>, model: String }`.
- `ChatResponse { message: Message, finish_reason: FinishReason }`; `FinishReason` enum: `Stop`, `ToolCalls`, `Length`.
- `StreamChunk { delta: Option<String>, tool_call_delta: Option<ToolCall>, finish_reason: Option<FinishReason> }` for the streaming variant.
- `ProviderError` (`thiserror`): `RateLimited { retry_after: Option<Duration> }`, `Auth`, `Network(#[from] reqwest::Error)`, `Parse(String)`, `Other(String)` — matchable so `core` retries on `RateLimited`, aborts on `Auth`.
- `trait LlmProvider`: `async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError>` + `async fn chat_stream(&self, req: ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk, ProviderError>>, ProviderError>`. `#[async_trait]`.
- All types `Serialize`/`Deserialize` + `Clone`/`Debug` where sane — Groq/Gemini impls map their wire format to/from these.

## 3. `fs` tool (`tools/src/fs.rs`)
**Tags:** `tools`, `P0` · **Time:** 2-3h

Implement `Tool` for read-file, write-file, list-dir, using `ignore` to respect `.gitignore`. Depends on #1.

## 4. `bash` tool (`tools/src/bash.rs`)
**Tags:** `tools`, `P0` · **Time:** 3-4h

Implement `Tool` for shell exec via `tokio::process`. Needs a safety/validation pass (deny-list or confirmation hook) before it's safe to wire into an autonomous loop — don't ship a raw unrestricted shell tool. Depends on #1.

## 5. Groq provider (`provider/src/groq.rs`)
**Tags:** `provider`, `P0` · **Time:** 3-4h

Reference `LlmProvider` impl — Groq's OpenAI-compatible REST API, simplest of the three (free, no local setup, standard tool-calling format). `reqwest` + SSE streaming. Depends on #2.

## 6. Gemini provider (`provider/src/gemini.rs`)
**Tags:** `provider`, `P1` · **Time:** 4-5h

Second `LlmProvider` impl — Google's API shape differs enough (content/parts format, different tool-call schema) to be the real test that the trait abstraction holds. Depends on #2, do after #5 lands.

## 7. Ollama provider (local models)
**Tags:** `provider`, `P2` · **Time:** 3-4h

Third impl, local-model support — no README/PROJECT.md file listed yet under `provider/src/`, will need a new `ollama.rs`. Lower priority: nice-to-have for offline use, not needed for first working demo. Depends on #2.

## 8. Agent ReAct loop (`core/src/agent.rs`)
**Tags:** `core`, `P0` · **Time:** 6-8h

The actual brain: loop of (LLM call → parse tool-call(s) → dispatch to `tools` → feed result back → repeat until final answer or max-iterations). Needs error branching on `ProviderError` variants (retry on rate-limit, abort on auth). Depends on #1-#5 (needs at least one real provider + the two tools to be a meaningful loop, not just #2/#1 stubs).

## 9. Conversation context/history (`core/src/context.rs`)
**Tags:** `core`, `P1` · **Time:** 3-4h

Message history buffer, token tracking/counting, truncation strategy when context grows past provider limits. Needed by #8 but can be built in parallel — stub a simple `Vec<Message>` first, revisit truncation once real token counts are visible from #5.

## 10. CLI arg parsing (`cli/src/args.rs`)
**Tags:** `cli`, `P1` · **Time:** 1-2h

`clap` derive struct: subcommands (e.g. `run`, `config`), provider selection flag, model flag, API key handling (env var vs config file). Independent of #8/#9, can start anytime.

## 11. CLI entrypoint + UI (`cli/src/main.rs`)
**Tags:** `cli`, `P1` · **Time:** 4-5h

Wire `args` → config/provider init → `cliclack` intro/spinner → kick off `core::agent`. This is the integration point — first time all 4 crates compile together into a working binary. Depends on #8, #9, #10.

## 12. Config / API key management
**Tags:** `cli`, `core`, `P1` · **Time:** 2-3h

Where do API keys live — env vars only, or a config file (`~/.config/granite/config.toml`)? Needs a decision before #11 can be considered done. No file currently allocated for this; likely lands in `cli` as a new module.

## 13. Streaming output in terminal
**Tags:** `cli`, `provider`, `P2` · **Time:** 3-4h

Wire SSE streaming (already in `reqwest` features + provider design) through to live token-by-token terminal output via `cliclack`. Polish item — functional non-streaming loop should work first.

## 14. Bash tool safety hardening
**Tags:** `tools`, `security`, `P1` · **Time:** 3-5h

Follow-up to #4: confirmation prompts before destructive commands, working-dir sandboxing, command allow/deny patterns. Should land before this tool is exposed to untrusted/complex prompts — flagged separately from #4 since "make it work" and "make it safe" are different bars.

## 15. Basic test coverage — `provider` + `tools`
**Tags:** `testing`, `P2` · **Time:** 4-6h

Unit tests per tool (`fs`, `bash`) and provider (mock HTTP for Groq/Gemini). None exist yet since crates are stubs; open once #3-#7 have real implementations to test.

## 16. README — usage docs
**Tags:** `docs`, `P2` · **Time:** 1-2h

Currently just the project name. Fill in once #11 gives a working binary — install steps (`nix develop`, `cargo build`), example run, supported providers, config format from #12.

---

### Suggested milestone grouping
- **Milestone "Walking skeleton"**: #1, #2, #3, #5, #8 (minimal loop: one provider, one tool, no error polish)
- **Milestone "MVP CLI"**: #4, #6, #9, #10, #11, #12
- **Milestone "Polish"**: #7, #13, #14, #15, #16
smoke test 1788861561
