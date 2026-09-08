use provider::Message;

#[derive(Debug, Default, Clone)]
pub struct ConversationContext {
    messages: Vec<Message>,
}

impl ConversationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

#[cfg(test)]
#[path = "context_test.rs"]
mod tests;
