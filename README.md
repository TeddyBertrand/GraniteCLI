# GraniteCLI

Rust CLI agent (ReAct loop) driving free/open LLM providers — Groq, Gemini, Ollama, Mistral. Reads/writes files and runs shell commands on your local dev environment.

## Install

Requires Nix (for the pinned toolchain) or a Rust toolchain matching `rust-toolchain`/`Cargo.toml`.

```sh
git clone https://github.com/TeddyBertrand/GraniteCLI
cd GraniteCLI
nix develop        # drops into devshell with cargo/rustc pinned
cargo build --release
```

Binary lands at `target/release/granite`.

## Usage

Interactive:

```sh
granite
```

Non-interactive (single prompt):

```sh
granite "list the files in this repo and summarize the project structure"
```

Pick a provider/model:

```sh
granite --provider gemini --model gemini-2.0-flash "explain this error"
```

View/set config:

```sh
granite config show
granite config set groq.api_key <key>
```

## Providers

| Provider | Flag value  | API key env var  | Default model source                  |
|----------|-------------|-------------------|----------------------------------------|
| Groq     | `groq`      | `GROQ_API_KEY`    | `provider::groq::DEFAULT_MODEL`        |
| Gemini   | `gemini`    | `GEMINI_API_KEY`  | `provider::gemini::DEFAULT_MODEL`      |
| Ollama   | `ollama`    | none (local)      | model must be running locally          |
| Mistral  | `mistral`   | `MISTRAL_API_KEY` | `provider::mistral::DEFAULT_MODEL`     |

Default provider: `groq`.

API key resolution order: `--api-key` flag > provider env var > config file (`<provider>.api_key`).

## Config format

Config file lives at `$XDG_CONFIG_HOME/granite/config.toml`, or `~/.config/granite/config.toml` if `XDG_CONFIG_HOME` is unset. TOML, created on first `config set`:

```toml
default_provider = "groq"
default_model = "llama-3.3-70b-versatile"

[groq]
api_key = "gsk_..."

[gemini]
api_key = "..."

[ollama]
api_key = ""

[mistral]
api_key = "..."
```

Settable keys via `granite config set <key> <value>`: `default_provider`, `default_model`, `groq.api_key`, `gemini.api_key`, `ollama.api_key`, `mistral.api_key`.
