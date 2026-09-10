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
    let outcome = registry.dispatch("/exit");
    assert!(matches!(outcome, Some(CommandOutcome::Exit)));
}

#[test]
fn dispatch_quit_alias_returns_exit_outcome() {
    let registry = default_registry();
    let outcome = registry.dispatch("/quit");
    assert!(matches!(outcome, Some(CommandOutcome::Exit)));
}

#[test]
fn dispatch_unknown_command_returns_none() {
    let registry = default_registry();
    assert!(registry.dispatch("/bogus").is_none());
}

#[test]
fn dispatch_non_command_line_returns_none() {
    let registry = default_registry();
    assert!(registry.dispatch("hello").is_none());
}

#[test]
fn dispatch_provider_switches_to_named_provider() {
    let registry = default_registry();
    let outcome = registry.dispatch("/provider gemini");
    assert!(matches!(
        outcome,
        Some(CommandOutcome::SwitchProvider(Provider::Gemini))
    ));
}

#[test]
fn dispatch_provider_missing_arg_is_info() {
    let registry = default_registry();
    let outcome = registry.dispatch("/provider");
    assert!(matches!(outcome, Some(CommandOutcome::Info(_))));
}

#[test]
fn dispatch_provider_unknown_name_is_info() {
    let registry = default_registry();
    let outcome = registry.dispatch("/provider bogus");
    assert!(matches!(outcome, Some(CommandOutcome::Info(_))));
}

#[test]
fn dispatch_model_switches_to_named_model() {
    let registry = default_registry();
    let outcome = registry.dispatch("/model llama-3.1-70b");
    assert!(matches!(
        outcome,
        Some(CommandOutcome::SwitchModel(m)) if m == "llama-3.1-70b"
    ));
}

#[test]
fn dispatch_model_missing_arg_is_info() {
    let registry = default_registry();
    let outcome = registry.dispatch("/model");
    assert!(matches!(outcome, Some(CommandOutcome::Info(_))));
}
