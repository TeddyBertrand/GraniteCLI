mod args;

use std::sync::Arc;

use anyhow::{bail, Context};
use clap::Parser;
use granite_core::agent::Agent;
use provider::LlmProvider;
use tools::bash::BashTool;
use tools::fs::{ListDirTool, ReadFileTool, WriteFileTool};

use args::{Cli, Commands, ConfigAction, Provider, RunArgs};

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
            ConfigAction::Show | ConfigAction::Set { .. } => {
                bail!("config command not implemented yet")
            }
        },
    }
}

fn build_provider(args: &RunArgs) -> anyhow::Result<(Arc<dyn LlmProvider>, String)> {
    let provider = args.provider.unwrap_or(Provider::Groq);

    match provider {
        Provider::Groq => {
            let groq = match &args.api_key {
                Some(key) => provider::groq::GroqProvider::new(key.clone()),
                None => provider::groq::GroqProvider::from_env()
                    .context("no Groq API key: pass --api-key or set GROQ_API_KEY")?,
            };
            let model = args
                .model
                .clone()
                .unwrap_or_else(|| provider::groq::DEFAULT_MODEL.to_string());
            Ok((Arc::new(groq), model))
        }
        Provider::Gemini => {
            let gemini = match &args.api_key {
                Some(key) => provider::gemini::GeminiProvider::new(key.clone()),
                None => provider::gemini::GeminiProvider::from_env()
                    .context("no Gemini API key: pass --api-key or set GEMINI_API_KEY")?,
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
    let (provider, model) = build_provider(&args)?;

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
