mod args;
mod config;

use std::sync::Arc;

use anyhow::bail;
use clap::Parser;
use granite_core::agent::Agent;
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

fn build_provider(args: &RunArgs, cfg: &Config) -> anyhow::Result<(Arc<dyn LlmProvider>, String)> {
    let provider = args.provider.unwrap_or(Provider::Groq);

    match provider {
        Provider::Groq => {
            let groq = if let Some(key) = &args.api_key {
                provider::groq::GroqProvider::new(key.clone())
            } else if let Ok(groq) = provider::groq::GroqProvider::from_env() {
                groq
            } else if let Some(key) = cfg.api_key_for(provider) {
                provider::groq::GroqProvider::new(key.to_string())
            } else {
                bail!("no Groq API key: pass --api-key, set GROQ_API_KEY, or run `granite config set groq.api_key <key>`")
            };
            let model = args
                .model
                .clone()
                .unwrap_or_else(|| provider::groq::DEFAULT_MODEL.to_string());
            Ok((Arc::new(groq), model))
        }
        Provider::Gemini => {
            let gemini = if let Some(key) = &args.api_key {
                provider::gemini::GeminiProvider::new(key.clone())
            } else if let Ok(gemini) = provider::gemini::GeminiProvider::from_env() {
                gemini
            } else if let Some(key) = cfg.api_key_for(provider) {
                provider::gemini::GeminiProvider::new(key.to_string())
            } else {
                bail!("no Gemini API key: pass --api-key, set GEMINI_API_KEY, or run `granite config set gemini.api_key <key>`")
            };
            let model = args
                .model
                .clone()
                .unwrap_or_else(|| provider::gemini::DEFAULT_MODEL.to_string());
            Ok((Arc::new(gemini), model))
        }
        Provider::Ollama => bail!("Ollama provider not implemented yet"),
    }
}

async fn run(args: RunArgs) -> anyhow::Result<()> {
    let cfg = Config::load()?;
    let (provider, model) = build_provider(&args, &cfg)?;

    cliclack::intro("granite")?;

    let prompt = match &args.prompt {
        Some(p) => p.clone(),
        None => cliclack::input("Prompt").interact()?,
    };

    let mut agent = Agent::new(provider, model)
        .with_tool(Arc::new(ReadFileTool))
        .with_tool(Arc::new(WriteFileTool))
        .with_tool(Arc::new(ListDirTool))
        .with_tool(Arc::new(BashTool::new()));

    let spinner = cliclack::spinner();
    spinner.start("thinking...");
    let result = agent.run(prompt).await;
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
