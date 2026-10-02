use std::sync::Arc;

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use mistralrs::{
    GgufModelBuilder, IsqBits, Model, ModelBuilder, RequestBuilder, Response, ResponseOk,
    TextMessageRole, TokenSource,
};

use crate::config::{Config, ModelLocation};

use super::{Role, TokenStream, Turn};

/// A mistral.rs model loaded once for the process.
#[derive(Clone)]
pub struct InProcess {
    model: Arc<Model>,
    temperature: f64,
    max_tokens: Option<usize>,
}

impl InProcess {
    pub async fn load(config: &Config) -> Result<Self, String> {
        let model = match &config.location {
            ModelLocation::Auto(id) => {
                let mut builder = ModelBuilder::new(id).with_logging();
                builder = apply_token(builder, config.api_key.as_deref());
                builder = apply_isq(builder, config.isq);
                builder.build().await.map_err(|error| error.to_string())?
            }
            ModelLocation::Gguf { model_id, files } => {
                let mut builder = GgufModelBuilder::new(model_id, files.clone()).with_logging();
                builder = apply_gguf_token(builder, config.api_key.as_deref());
                builder.build().await.map_err(|error| error.to_string())?
            }
        };

        Ok(Self {
            model: Arc::new(model),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
        })
    }

    pub async fn stream(&self, turns: Vec<Turn>) -> Result<TokenStream, String> {
        let request = request_builder(&turns, self.temperature, self.max_tokens)?;
        // The mistral.rs stream borrows the model. Own the model in a task and
        // forward deltas so the caller can hold a `'static` stream. Dropping
        // that stream closes the channel and stops generation.
        let model = Arc::clone(&self.model);
        let (mut tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            let stream = match model.stream_chat_request(request).await {
                Ok(stream) => stream,
                Err(error) => {
                    let _ = tx.send(Err(error.to_string())).await;
                    return;
                }
            };
            let mut stream = std::pin::pin!(stream);
            while let Some(response) = stream.next().await {
                if tx.is_closed() {
                    break;
                }
                let item = match text_delta(response) {
                    Ok(Some(text)) => Ok(text),
                    Ok(None) => continue,
                    Err(error) => Err(error),
                };
                if tx.send(item).await.is_err() {
                    break;
                }
            }
        });
        Ok(Box::pin(rx))
    }
}

fn text_delta(response: Response) -> Result<Option<String>, String> {
    match response.as_result() {
        Ok(ResponseOk::Chunk(chunk)) => Ok(chunk
            .choices
            .first()
            .and_then(|choice| choice.delta.content.clone())
            .filter(|text| !text.is_empty())),
        Ok(_) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn request_builder(
    turns: &[Turn],
    temperature: f64,
    max_tokens: Option<usize>,
) -> Result<RequestBuilder, String> {
    if turns.is_empty() {
        return Err("Nothing to send to the model.".into());
    }

    let mut request = RequestBuilder::new().with_truncate_sequence(true);
    for turn in turns {
        if turn.text.is_empty() {
            continue;
        }
        let role = match turn.role {
            Role::System => TextMessageRole::System,
            Role::User => TextMessageRole::User,
            Role::Assistant => TextMessageRole::Assistant,
        };
        request = request.add_message(role, &turn.text);
    }

    request = if temperature <= 0.0 {
        request.set_deterministic_sampler()
    } else {
        request
            .set_sampler_temperature(temperature)
            .set_sampler_topp(0.95)
    };
    if let Some(max_tokens) = max_tokens {
        request = request.set_sampler_max_len(max_tokens);
    }
    Ok(request)
}

fn apply_token(builder: ModelBuilder, api_key: Option<&str>) -> ModelBuilder {
    match api_key {
        Some(key) => builder.with_token_source(TokenSource::Literal(key.to_owned())),
        None => builder,
    }
}

fn apply_gguf_token(builder: GgufModelBuilder, api_key: Option<&str>) -> GgufModelBuilder {
    match api_key {
        Some(key) => builder.with_token_source(TokenSource::Literal(key.to_owned())),
        None => builder,
    }
}

fn apply_isq(builder: ModelBuilder, isq: Option<IsqBits>) -> ModelBuilder {
    match isq {
        Some(bits) => builder.with_auto_isq(bits),
        None => builder,
    }
}
