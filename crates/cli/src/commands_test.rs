use super::*;

#[test]
fn parse_splits_name_and_args() {
    assert_eq!(CommandRegistry::parse("/exit"), Some(("exit", "")));
    assert_eq!(CommandRegistry::parse("/model gpt-4"), Some(("model", "gpt-4")));
    assert_eq!(CommandRegistry::parse("not a command"), None);
}

#[test]
fn dispatch_exit_returns_exit_outcome() {
    let registry = default_registry();
    let mut history = Vec::new();
    let outcome = registry.dispatch("/exit", &mut history);
    assert!(matches!(outcome, Some(CommandOutcome::Exit)));
}

#[test]
fn dispatch_quit_alias_returns_exit_outcome() {
    let registry = default_registry();
    let mut history = Vec::new();
    let outcome = registry.dispatch("/quit", &mut history);
    assert!(matches!(outcome, Some(CommandOutcome::Exit)));
}

#[test]
fn dispatch_unknown_command_returns_none() {
    let registry = default_registry();
    let mut history = Vec::new();
    assert!(registry.dispatch("/bogus", &mut history).is_none());
}

#[test]
fn dispatch_non_command_line_returns_none() {
    let registry = default_registry();
    let mut history = Vec::new();
    assert!(registry.dispatch("hello", &mut history).is_none());
}
