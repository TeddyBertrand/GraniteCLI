use super::*;
use provider::Role;

fn msg(text: &str) -> Message {
    Message {
        role: Role::User,
        content: Some(text.to_string()),
        tool_calls: vec![],
        tool_call_id: None,
    }
}

#[test]
fn push_appends_in_order() {
    let mut ctx = ConversationContext::new();
    ctx.push(msg("first"));
    ctx.push(msg("second"));

    assert_eq!(ctx.len(), 2);
    assert_eq!(ctx.messages()[0].content.as_deref(), Some("first"));
    assert_eq!(ctx.messages()[1].content.as_deref(), Some("second"));
}

#[test]
fn new_context_is_empty() {
    let ctx = ConversationContext::new();
    assert!(ctx.is_empty());
    assert_eq!(ctx.len(), 0);
}

#[test]
fn clear_empties_buffer() {
    let mut ctx = ConversationContext::new();
    ctx.push(msg("hi"));
    ctx.clear();
    assert!(ctx.is_empty());
}
