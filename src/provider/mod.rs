mod inprocess;
mod ollama;

use std::pin::Pin;

use futures::Stream;

use crate::config::Config;

pub use inprocess::InProcess;
pub use ollama::OllamaChat;

/// One turn sent to a backend. The system turn, when present, is first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>;

#[derive(Clone)]
pub enum Backend {
    InProcess(InProcess),
    Ollama(OllamaChat),
}

impl Backend {
    pub async fn connect(config: &Config) -> Result<Self, String> {
        match config.provider {
            crate::config::ProviderKind::InProcess => {
                Ok(Self::InProcess(InProcess::load(config).await?))
            }
            crate::config::ProviderKind::Ollama => Ok(Self::Ollama(OllamaChat::connect(config)?)),
        }
    }

    pub async fn stream(&self, turns: Vec<Turn>) -> Result<TokenStream, String> {
        match self {
            Self::InProcess(backend) => backend.stream(turns).await,
            Self::Ollama(backend) => backend.stream(turns),
        }
    }
}
