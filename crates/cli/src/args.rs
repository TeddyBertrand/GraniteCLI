use clap::{Parser, Subcommand, ValueEnum};

/// GraniteCLI — ReAct agent driving free/open LLM providers.
#[derive(Debug, Parser)]
#[command(name = "granite", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start an agent run (default when no subcommand given).
    Run(RunArgs),
    /// View or set configuration.
    Config(ConfigArgs),
}

#[derive(Debug, Parser)]
pub struct RunArgs {
    /// LLM provider to use.
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,

    /// Model name override (defaults to the provider's own default model).
    #[arg(long)]
    pub model: Option<String>,

    /// API key. Precedence: this flag > provider-specific env var (e.g. GROQ_API_KEY) > config file.
    #[arg(long)]
    pub api_key: Option<String>,

    /// Prompt to run non-interactively; omit for interactive mode.
    pub prompt: Option<String>,
}

#[derive(Debug, Parser)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Print current configuration.
    Show,
    /// Set a configuration value.
    Set {
        key: String,
        value: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Provider {
    Groq,
    Gemini,
    Ollama,
}
