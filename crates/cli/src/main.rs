mod args;
mod commands;
mod config;
mod tui;

use std::sync::Arc;

use anyhow::bail;
use async_trait::async_trait;
use clap::Parser;
use granite_core::agent::Agent;
use granite_core::{ConfirmHook, ConfirmRequest};
use provider::LlmProvider;
use tools::bash::BashTool;
use tools::fs::{ListDirTool, ReadFileTool, WriteFileTool};

use args::{Cli, Commands, ConfigAction, Provider, RunArgs};
use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Run(RunArgs {
        provider: None,
        model: None,
        api_key: None,
        prompt: None,
    })) {
        Commands::Run(args) => run(args).await,
        Commands::Config(args) => match args.action {
            ConfigAction::Show => show_config(),
            ConfigAction::Set { key, value } => set_config(&key, &value),
        },
    }
}

fn show_config() -> anyhow::Result<()> {
    let cfg = Config::load()?;
    println!("default_provider = {:?}", cfg.default_provider);
    println!("default_model = {:?}", cfg.default_model);
    println!(
        "groq.api_key = {}",
        cfg.groq
            .api_key
            .as_deref()
            .map(config::mask)
            .unwrap_or_else(|| "<unset>".to_string())
    );
    println!(
        "gemini.api_key = {}",
        cfg.gemini
            .api_key
            .as_deref()
            .map(config::mask)
            .unwrap_or_else(|| "<unset>".to_string())
    );
    println!(
        "ollama.api_key = {}",
        cfg.ollama
            .api_key
            .as_deref()
            .map(config::mask)
            .unwrap_or_else(|| "<unset>".to_string())
    );
    Ok(())
}

fn set_config(key: &str, value: &str) -> anyhow::Result<()> {
    let mut cfg = Config::load()?;
    cfg.set(key, value)?;
    cfg.save()?;
    println!("set {key}");
    Ok(())
}

/// Builds a provider + model pair. `Ok(None)` means the (Groq) provider is
/// selected but no key could be resolved from arg/env/config — interactive
/// mode can recover from this via the TUI's API key setup; non-interactive
/// (`--prompt`) mode must still fail fast on it. `api_key` precedence:
/// explicit arg > provider-specific env var > config file.
pub(crate) fn build_provider(
    provider: Provider,
    model: Option<String>,
    api_key: Option<String>,
    cfg: &Config,
) -> anyhow::Result<Option<(Arc<dyn LlmProvider>, String)>> {
    match provider {
        Provider::Groq => {
            let groq = if let Some(key) = &api_key {
                Some(provider::groq::GroqProvider::new(key.clone()))
            } else if let Ok(groq) = provider::groq::GroqProvider::from_env() {
                Some(groq)
            } else {
                cfg.api_key_for(provider)
                    .map(|key| provider::groq::GroqProvider::new(key.to_string()))
            };
            let model = model.unwrap_or_else(|| provider::groq::DEFAULT_MODEL.to_string());
            Ok(groq.map(|g| (Arc::new(g) as Arc<dyn LlmProvider>, model)))
        }
        Provider::Gemini => bail!("Gemini provider not implemented yet"),
        Provider::Ollama => bail!("Ollama provider not implemented yet"),
    }
}

async fn run(args: RunArgs) -> anyhow::Result<()> {
    let cfg = Config::load()?;
    let mut provider_kind = args.provider.unwrap_or(Provider::Groq);
    let resolved = build_provider(provider_kind, args.model.clone(), args.api_key.clone(), &cfg)?;

    match &args.prompt {
        Some(p) => {
            let Some((provider, model)) = resolved else {
                bail!(
                    "no {} API key: pass --api-key, set the provider's env var, or run `granite config set {}.api_key <key>`",
                    provider_kind.label(),
                    provider_kind.config_prefix()
                );
            };
            let mut agent =
                build_agent(Agent::new(provider, model)).with_confirm_hook(Arc::new(OneShotConfirmHook));

            let _alt_screen = tui::AltScreenGuard::enter()?;
            cliclack::intro("granite")?;
            let spinner = cliclack::spinner();
            spinner.start("thinking...");
            let result = agent.run(p.clone()).await;
            spinner.stop("done");

            match result {
                Ok(answer) => {
                    cliclack::outro(answer)?;
                    Ok(())
                }
                Err(err) => {
                    cliclack::outro(format!("error: {err}"))?;
                    Err(err.into())
                }
            }
        }
        None => {
            let model = args
                .model
                .clone()
                .unwrap_or_else(|| provider::groq::DEFAULT_MODEL.to_string());
            let mut agent = match resolved {
                Some((provider, model)) => build_agent(Agent::new(provider, model)),
                None => build_agent(Agent::without_provider(model)),
            };

            let _alt_screen = tui::AltScreenGuard::enter()?;
            tui::run_chat_loop(&mut agent, &cfg, &mut provider_kind).await
        }
    }
}

/// Builds a provider from an explicit key, e.g. one just typed into the
/// TUI's API key setup screen.
pub(crate) fn provider_from_key(provider: Provider, key: String) -> anyhow::Result<Arc<dyn LlmProvider>> {
    match provider {
        Provider::Groq => Ok(Arc::new(provider::groq::GroqProvider::new(key))),
        Provider::Gemini => bail!("Gemini provider not implemented yet"),
        Provider::Ollama => bail!("Ollama provider not implemented yet"),
    }
}

struct OneShotConfirmHook;

#[async_trait]
impl ConfirmHook for OneShotConfirmHook {
    async fn confirm(&self, request: ConfirmRequest) -> bool {
        let prompt = format!(
            "{} wants to run ({:?}): {} — allow?",
            request.tool_name, request.risk, request.detail
        );
        tokio::task::spawn_blocking(move || {
            cliclack::confirm(prompt).initial_value(false).interact().unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }
}

fn build_agent(agent: Agent) -> Agent {
    agent
        .with_tool(Arc::new(ReadFileTool))
        .with_tool(Arc::new(WriteFileTool))
        .with_tool(Arc::new(ListDirTool))
        .with_tool(Arc::new(BashTool::new()))
}
