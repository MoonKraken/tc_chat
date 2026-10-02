use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures::StreamExt;
use tokio::sync::broadcast;

use crate::provider::{Backend, Role, Turn};

const MAX_PROMPT_CHARS: usize = 8_000;
const MAX_REPLY_CHARS: usize = 32_000;
const MAX_STORED_MESSAGES: usize = 200;
const MODEL_HISTORY: usize = 40;
const FLUSH_AFTER: Duration = Duration::from_millis(40);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Author {
    User,
    Assistant,
}

struct StoredMessage {
    id: u64,
    author: Author,
    text: String,
    pending: bool,
}

struct ConversationInner {
    messages: Vec<StoredMessage>,
    next_id: u64,
    generating: bool,
}

/// One browser conversation. Token appends notify `updated`.
pub struct Conversation {
    inner: Mutex<ConversationInner>,
    updated: broadcast::Sender<()>,
}

#[derive(Clone)]
pub struct ChatLine {
    pub id: u64,
    pub mine: bool,
    pub text: String,
    pub pending: bool,
}

pub struct ChatHub {
    conversations: Mutex<HashMap<String, Arc<Conversation>>>,
    backend: Arc<Backend>,
    system_prompt: String,
}

struct Prepared {
    conversation: Arc<Conversation>,
    assistant_id: u64,
    turns: Vec<Turn>,
}

impl ChatHub {
    pub fn new(backend: Backend, system_prompt: String) -> Self {
        Self {
            conversations: Mutex::new(HashMap::new()),
            backend: Arc::new(backend),
            system_prompt,
        }
    }

    pub fn open(&self, id: &str) -> Arc<Conversation> {
        let mut conversations = self
            .conversations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conversations
            .entry(id.to_owned())
            .or_insert_with(Conversation::new)
            .clone()
    }

    /// Accepts `text` for one page's conversation and runs generation until it
    /// finishes. The returned error string is for the composer. An empty
    /// string means the reply completed.
    pub async fn respond(&self, id: &str, text: &str) -> Result<(), String> {
        let prepared = self.prepare(id, text)?;
        let backend = Arc::clone(&self.backend);
        // Spawned so a disconnected caller does not cancel the reply. Dropping
        // the join handle detaches the task.
        tokio::spawn(generate(
            backend,
            prepared.conversation,
            prepared.assistant_id,
            prepared.turns,
        ))
        .await
        .map_err(|_| "The reply stopped unexpectedly.".to_owned())
    }

    fn prepare(&self, id: &str, text: &str) -> Result<Prepared, String> {
        let text = normalize_prompt(text)?;
        let conversation = self.open(id);
        let mut inner = conversation.lock();
        if inner.generating {
            return Err("Wait for the current reply to finish.".into());
        }

        inner.generating = true;
        inner.push(Author::User, text, false);
        let assistant_id = inner.push(Author::Assistant, String::new(), true);
        inner.truncate();
        let turns = inner.turns(&self.system_prompt);
        drop(inner);
        conversation.notify_updated();

        Ok(Prepared {
            conversation,
            assistant_id,
            turns,
        })
    }
}

impl Conversation {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ConversationInner {
                messages: Vec::new(),
                next_id: 1,
                generating: false,
            }),
            updated: broadcast::channel(64).0,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConversationInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn subscribe_updated(&self) -> broadcast::Receiver<()> {
        self.updated.subscribe()
    }

    pub fn lines(&self) -> Vec<ChatLine> {
        self.lock()
            .messages
            .iter()
            .map(|message| ChatLine {
                id: message.id,
                mine: message.author == Author::User,
                text: message.text.clone(),
                pending: message.pending,
            })
            .collect()
    }

    fn append(&self, id: u64, text: &str) -> bool {
        let mut inner = self.lock();
        let Some(message) = inner.messages.iter_mut().find(|message| message.id == id) else {
            return false;
        };
        if message.text.chars().count() >= MAX_REPLY_CHARS {
            return false;
        }
        message.text.push_str(text);
        if message.text.chars().count() > MAX_REPLY_CHARS {
            let end = message
                .text
                .char_indices()
                .nth(MAX_REPLY_CHARS)
                .map(|(index, _)| index)
                .unwrap_or(message.text.len());
            message.text.truncate(end);
            message.text.push_str("\n\n[Reply truncated.]");
            return false;
        }
        true
    }

    fn finish(&self, id: u64, failure: Option<&str>) {
        let mut inner = self.lock();
        if let Some(message) = inner.messages.iter_mut().find(|message| message.id == id) {
            match failure {
                Some(failure) if message.text.is_empty() => message.text = failure.to_owned(),
                Some(failure) => {
                    message.text.push_str("\n\n");
                    message.text.push_str(failure);
                }
                None if message.text.is_empty() => {
                    message.text = "The model returned an empty reply.".to_owned();
                }
                None => {}
            }
            message.pending = false;
        }
        inner.generating = false;
    }

    fn notify_updated(&self) {
        let _ = self.updated.send(());
    }
}

impl ConversationInner {
    fn push(&mut self, author: Author, text: String, pending: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.messages.push(StoredMessage {
            id,
            author,
            text,
            pending,
        });
        id
    }

    fn truncate(&mut self) {
        if self.messages.len() > MAX_STORED_MESSAGES {
            let extra = self.messages.len() - MAX_STORED_MESSAGES;
            self.messages.drain(0..extra);
        }
    }

    fn turns(&self, system_prompt: &str) -> Vec<Turn> {
        let mut turns = Vec::new();
        if !system_prompt.is_empty() {
            turns.push(Turn {
                role: Role::System,
                text: system_prompt.to_owned(),
            });
        }

        let visible: Vec<_> = self
            .messages
            .iter()
            .filter(|message| !message.text.is_empty())
            .collect();
        let start = visible.len().saturating_sub(MODEL_HISTORY);
        let mut window = &visible[start..];
        while window
            .first()
            .is_some_and(|message| message.author == Author::Assistant)
        {
            window = &window[1..];
        }
        for message in window {
            turns.push(Turn {
                role: match message.author {
                    Author::User => Role::User,
                    Author::Assistant => Role::Assistant,
                },
                text: message.text.clone(),
            });
        }
        turns
    }
}

async fn generate(
    backend: Arc<Backend>,
    conversation: Arc<Conversation>,
    assistant_id: u64,
    turns: Vec<Turn>,
) {
    let mut fallback = None;
    match backend.stream(turns).await {
        Ok(mut stream) => {
            let mut last_flush = Instant::now() - FLUSH_AFTER;
            let mut truncated = false;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        if !conversation.append(assistant_id, &text) {
                            truncated = true;
                            break;
                        }
                        if last_flush.elapsed() >= FLUSH_AFTER {
                            conversation.notify_updated();
                            last_flush = Instant::now();
                        }
                    }
                    Err(error) => {
                        fallback = Some(format!("The model stopped: {error}"));
                        break;
                    }
                }
            }
            if truncated {
                // Dropping the stream cancels the rest of the completion.
                drop(stream);
            }
        }
        Err(error) => fallback = Some(format!("The model stopped: {error}")),
    }

    conversation.finish(assistant_id, fallback.as_deref());
    conversation.notify_updated();
}

pub fn normalize_prompt(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Write a message first.".into());
    }
    let count = text.chars().count();
    if count > MAX_PROMPT_CHARS {
        return Err(format!(
            "Keep messages under {MAX_PROMPT_CHARS} characters."
        ));
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_trimmed_and_bounded() {
        assert_eq!(normalize_prompt("  hello ").unwrap(), "hello");
        assert!(normalize_prompt("   ").is_err());
        let huge = "é".repeat(MAX_PROMPT_CHARS + 1);
        assert!(normalize_prompt(&huge).is_err());
    }
}
