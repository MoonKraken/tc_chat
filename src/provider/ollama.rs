use futures::{StreamExt, stream};
use rig::Model;
use rig::completion::CompletionRequest;
use rig::message::Message;
use rig::providers::ollama::{Chat, OllamaConfig};
use rig::streaming::{Item, StreamEvent};

use crate::config::Config;

use super::{Role, TokenStream, Turn};

/// Ollama chat through rig. The model handle is cheap to clone.
#[derive(Clone)]
pub struct OllamaChat {
    model: Model<Chat>,
    temperature: f64,
    max_tokens: Option<u64>,
}

impl OllamaChat {
    pub fn connect(config: &Config) -> Result<Self, String> {
        let mut ollama = OllamaConfig::new().with_base_url(&config.provider_base);
        if let Some(api_key) = &config.api_key {
            ollama = ollama.with_api_key(api_key.clone());
        }

        Ok(Self {
            model: ollama.client().completion(&config.model),
            temperature: config.temperature,
            max_tokens: config.max_tokens.map(|tokens| tokens as u64),
        })
    }

    pub fn stream(&self, turns: Vec<Turn>) -> Result<TokenStream, String> {
        let request = completion_request(&turns, self.temperature, self.max_tokens)?;
        let streamed = self
            .model
            .stream(request)
            .map_err(|error| error.to_string())?;

        Ok(Box::pin(streamed.filter_map(|item| async move {
            match item {
                Ok(Item::Event(StreamEvent::Text { text, .. })) if !text.is_empty() => {
                    Some(Ok(text))
                }
                Ok(Item::Event(_) | Item::Unknown(_)) => None,
                Err(error) => Some(Err(error.to_string())),
            }
        })))
    }
}

fn completion_request(
    turns: &[Turn],
    temperature: f64,
    max_tokens: Option<u64>,
) -> Result<CompletionRequest, String> {
    let (prompt, earlier) = split_prompt(turns)?;
    let mut history = Vec::with_capacity(earlier.len());
    let mut system = None;

    for turn in earlier {
        match turn.role {
            Role::System if system.is_none() && history.is_empty() => {
                system = Some(turn.text.clone());
            }
            Role::System => {
                return Err("The system prompt has to be the first message.".into());
            }
            Role::User => history.push(Message::user(&turn.text)),
            Role::Assistant => history.push(Message::assistant(&turn.text)),
        }
    }

    let mut request = CompletionRequest::new(Message::user(prompt)).messages(history);
    if let Some(system) = system {
        request = request.preamble(system);
    }
    if temperature > 0.0 {
        request = request.temperature(temperature);
    }
    if let Some(max_tokens) = max_tokens {
        request = request.max_tokens(max_tokens);
    }
    Ok(request)
}

fn split_prompt(turns: &[Turn]) -> Result<(&str, &[Turn]), String> {
    let Some((last, earlier)) = turns.split_last() else {
        return Err("Nothing to send to the model.".into());
    };
    if last.role != Role::User || last.text.is_empty() {
        return Err("The latest message has to be from you.".into());
    }
    Ok((&last.text, earlier))
}

#[allow(dead_code)]
fn _stream_is_send(chat: &OllamaChat, turns: Vec<Turn>) {
    fn assert_send<T: Send>(_: &T) {}
    if let Ok(stream) = chat.stream(turns) {
        assert_send(&stream);
    }
    let _ = stream::empty::<Result<String, String>>();
}
