use std::collections::HashMap;
use std::sync::Arc;

/// Result of dispatching a command, signalling what `run_chat_loop` should
/// do next.
pub enum CommandOutcome {
    Exit,
}

pub trait SlashCommand: Send + Sync {
    /// Canonical name, without leading '/', e.g. "exit".
    fn name(&self) -> &str;
    /// Additional names that resolve to this same command, e.g. ["quit"].
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn execute(&self) -> CommandOutcome;
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
        let (name, _args) = Self::parse(line)?;
        let command = self.commands.get(name)?;
        Some(command.execute())
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

    fn execute(&self) -> CommandOutcome {
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
