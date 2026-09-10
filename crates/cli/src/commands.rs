use std::collections::HashMap;
use std::sync::Arc;

use clap::ValueEnum;

use crate::args::Provider;

/// Result of dispatching a command, signalling what `run_chat_loop` should
/// do next.
pub enum CommandOutcome {
    Exit,
    /// Switch to a different provider (model resets to that provider's default).
    SwitchProvider(Provider),
    /// Switch model, keeping the current provider.
    SwitchModel(String),
    /// Informational or error message to show in the history panel.
    Info(String),
}

pub trait SlashCommand: Send + Sync {
    /// Canonical name, without leading '/', e.g. "exit".
    fn name(&self) -> &str;
    /// Additional names that resolve to this same command, e.g. ["quit"].
    fn aliases(&self) -> &[&str] {
        &[]
    }
    /// `args` is the rest of the line after the command name, trimmed.
    fn execute(&self, args: &str) -> CommandOutcome;
}

pub struct CommandRegistry {
    commands: HashMap<String, Arc<dyn SlashCommand>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    pub fn register(&mut self, command: Arc<dyn SlashCommand>) {
        self.commands
            .insert(command.name().to_string(), Arc::clone(&command));
        for alias in command.aliases() {
            self.commands.insert((*alias).to_string(), Arc::clone(&command));
        }
    }

    /// Splits `"/name rest of line"` into (name-without-slash, rest-trimmed).
    /// Returns `None` if `line` doesn't start with '/'.
    pub fn parse(line: &str) -> Option<(&str, &str)> {
        let rest = line.strip_prefix('/')?;
        Some(match rest.split_once(char::is_whitespace) {
            Some((name, args)) => (name, args.trim_start()),
            None => (rest, ""),
        })
    }

    /// Looks up and runs the command named by `line`. Returns `None` if
    /// `line` isn't a recognized slash command (caller decides how to
    /// report "unknown command" vs. falling through to the agent).
    pub fn dispatch(&self, line: &str) -> Option<CommandOutcome> {
        let (name, args) = Self::parse(line)?;
        let command = self.commands.get(name)?;
        Some(command.execute(args))
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// `/exit` and `/quit` both terminate the chat loop.
pub struct ExitCommand;

impl SlashCommand for ExitCommand {
    fn name(&self) -> &str {
        "exit"
    }

    fn aliases(&self) -> &[&str] {
        &["quit"]
    }

    fn execute(&self, _args: &str) -> CommandOutcome {
        CommandOutcome::Exit
    }
}

/// `/provider <groq|gemini|ollama>` switches provider (and resets model to
/// that provider's default).
pub struct ProviderCommand;

impl SlashCommand for ProviderCommand {
    fn name(&self) -> &str {
        "provider"
    }

    fn execute(&self, args: &str) -> CommandOutcome {
        let name = args.trim();
        if name.is_empty() {
            return CommandOutcome::Info("usage: /provider <groq|gemini|ollama>".to_string());
        }
        match Provider::from_str(name, true) {
            Ok(provider) => CommandOutcome::SwitchProvider(provider),
            Err(_) => CommandOutcome::Info(format!(
                "unknown provider '{name}' — expected one of: groq, gemini, ollama"
            )),
        }
    }
}

/// `/model <name>` switches model, keeping the current provider.
pub struct ModelCommand;

impl SlashCommand for ModelCommand {
    fn name(&self) -> &str {
        "model"
    }

    fn execute(&self, args: &str) -> CommandOutcome {
        let name = args.trim();
        if name.is_empty() {
            return CommandOutcome::Info("usage: /model <name>".to_string());
        }
        CommandOutcome::SwitchModel(name.to_string())
    }
}

pub fn default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(Arc::new(ExitCommand));
    registry.register(Arc::new(ProviderCommand));
    registry.register(Arc::new(ModelCommand));
    registry
}

#[cfg(test)]
#[path = "commands_test.rs"]
mod tests;
