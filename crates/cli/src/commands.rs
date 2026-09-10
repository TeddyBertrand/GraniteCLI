use std::collections::HashMap;
use std::sync::Arc;

use crate::tui::HistoryEntry;

/// What a command handler is allowed to do to the running chat loop.
/// `ExitCommand` doesn't need these yet; future commands (`/provider`,
/// `/model`) will read `args` and push to `history`.
#[allow(dead_code)]
pub struct CommandContext<'a> {
    pub history: &'a mut Vec<HistoryEntry>,
    pub args: &'a str,
}

/// Result of dispatching a command: either it was handled in place, or it
/// signals that `run_chat_loop` should break out of its loop. `Handled` has
/// no producer yet — `ExitCommand` is the only command so far.
#[allow(dead_code)]
pub enum CommandOutcome {
    Handled,
    Exit,
}

pub trait SlashCommand: Send + Sync {
    /// Canonical name, without leading '/', e.g. "exit".
    fn name(&self) -> &str;
    /// Additional names that resolve to this same command, e.g. ["quit"].
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn execute(&self, ctx: CommandContext<'_>) -> CommandOutcome;
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
    pub fn dispatch(&self, line: &str, history: &mut Vec<HistoryEntry>) -> Option<CommandOutcome> {
        let (name, args) = Self::parse(line)?;
        let command = self.commands.get(name)?;
        Some(command.execute(CommandContext { history, args }))
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

    fn execute(&self, _ctx: CommandContext<'_>) -> CommandOutcome {
        CommandOutcome::Exit
    }
}

pub fn default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(Arc::new(ExitCommand));
    registry
}

#[cfg(test)]
#[path = "commands_test.rs"]
mod tests;
