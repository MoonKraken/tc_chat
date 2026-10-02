use std::{env, fmt, path::Path};

use mistralrs::IsqBits;

const DEFAULT_OLLAMA_BASE: &str = "http://127.0.0.1:11434";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant. Answer clearly and concisely.";
const DEFAULT_TEMPERATURE: f64 = 0.7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    InProcess,
    Ollama,
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InProcess => "inprocess",
            Self::Ollama => "ollama",
        })
    }
}

/// Where mistral.rs should load weights from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelLocation {
    /// Hugging Face id or a local directory of non-GGUF weights.
    Auto(String),
    /// A GGUF repo or directory plus the weight filenames.
    Gguf {
        model_id: String,
        files: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: ProviderKind,
    pub provider_base: String,
    pub api_key: Option<String>,
    pub model: String,
    pub location: ModelLocation,
    pub isq: Option<IsqBits>,
    pub system_prompt: String,
    pub temperature: f64,
    pub max_tokens: Option<usize>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let provider = parse_provider(&required("PROVIDER")?)?;
        let model = required("MODEL")?;
        let model_file = optional("MODEL_FILE");
        let provider_base = match optional("PROVIDER_BASE") {
            Some(base) => base.trim_end_matches('/').to_owned(),
            None => DEFAULT_OLLAMA_BASE.to_owned(),
        };
        let api_key = optional("API_KEY").filter(|key| !key.is_empty());
        let isq = match optional("ISQ") {
            Some(value) => Some(parse_isq(&value)?),
            None => None,
        };
        let system_prompt =
            optional("SYSTEM_PROMPT").unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_owned());
        let temperature = match optional("TEMPERATURE") {
            Some(value) => parse_temperature(&value)?,
            None => DEFAULT_TEMPERATURE,
        };
        let max_tokens = match optional("MAX_TOKENS") {
            Some(value) => Some(parse_max_tokens(&value)?),
            None => None,
        };

        if provider == ProviderKind::Ollama && provider_base.is_empty() {
            return Err(ConfigError(
                "PROVIDER_BASE is empty. Set it to the Ollama URL, for example http://127.0.0.1:11434.".into(),
            ));
        }

        let location = model_location(&model, model_file.as_deref())?;
        check_isq(provider, isq, &location)?;

        Ok(Self {
            provider,
            provider_base,
            api_key,
            location,
            model,
            isq,
            system_prompt,
            temperature,
            max_tokens,
        })
    }
}

pub fn parse_provider(value: &str) -> Result<ProviderKind, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inprocess" => Ok(ProviderKind::InProcess),
        "ollama" => Ok(ProviderKind::Ollama),
        other => Err(ConfigError(format!(
            "PROVIDER must be \"inprocess\" or \"ollama\", got {other:?}"
        ))),
    }
}

fn check_isq(
    provider: ProviderKind,
    isq: Option<IsqBits>,
    location: &ModelLocation,
) -> Result<(), ConfigError> {
    if isq.is_none() {
        return Ok(());
    }
    match (provider, location) {
        (ProviderKind::Ollama, _) => Err(ConfigError(
            "ISQ applies only when PROVIDER=inprocess.".into(),
        )),
        (ProviderKind::InProcess, ModelLocation::Gguf { .. }) => Err(ConfigError(
            "ISQ quantizes a full model as it loads. A GGUF file is already quantized, so leave ISQ unset.".into(),
        )),
        (ProviderKind::InProcess, ModelLocation::Auto(_)) => Ok(()),
    }
}

pub fn parse_isq(value: &str) -> Result<IsqBits, ConfigError> {
    match value.trim() {
        "2" => Ok(IsqBits::Two),
        "3" => Ok(IsqBits::Three),
        "4" => Ok(IsqBits::Four),
        "5" => Ok(IsqBits::Five),
        "6" => Ok(IsqBits::Six),
        "8" => Ok(IsqBits::Eight),
        other => Err(ConfigError(format!(
            "ISQ must be 2, 3, 4, 5, 6, or 8, got {other:?}"
        ))),
    }
}

fn parse_temperature(value: &str) -> Result<f64, ConfigError> {
    let temperature: f64 = value
        .trim()
        .parse()
        .map_err(|_| ConfigError(format!("TEMPERATURE must be a number, got {value:?}")))?;
    if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
        return Err(ConfigError("TEMPERATURE must be between 0 and 2.".into()));
    }
    Ok(temperature)
}

fn parse_max_tokens(value: &str) -> Result<usize, ConfigError> {
    let max_tokens: usize = value.trim().parse().map_err(|_| {
        ConfigError(format!(
            "MAX_TOKENS must be a positive integer, got {value:?}"
        ))
    })?;
    if max_tokens == 0 {
        return Err(ConfigError("MAX_TOKENS must be greater than 0.".into()));
    }
    Ok(max_tokens)
}

pub fn model_location(model: &str, model_file: Option<&str>) -> Result<ModelLocation, ConfigError> {
    if let Some(file) = model_file {
        if file.trim().is_empty() {
            return Err(ConfigError(
                "MODEL_FILE is set but empty. Unset it, or name the .gguf file.".into(),
            ));
        }
        return Ok(ModelLocation::Gguf {
            model_id: model.to_owned(),
            files: vec![file.to_owned()],
        });
    }

    let path = Path::new(model);
    if path.extension().and_then(|ext| ext.to_str()) == Some("gguf") {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ConfigError(format!("MODEL {model:?} is not a valid .gguf path")))?
            .to_owned();
        let model_id = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_owned());
        return Ok(ModelLocation::Gguf {
            model_id,
            files: vec![file],
        });
    }

    Ok(ModelLocation::Auto(model.to_owned()))
}

fn required(name: &str) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().to_owned()),
        Ok(_) => Err(ConfigError(format!("{name} is empty"))),
        Err(_) => Err(ConfigError(format!(
            "{name} is not set. Copy .env.example to .env and fill it in."
        ))),
    }
}

fn optional(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_match_the_env_contract() {
        assert_eq!(
            parse_provider("inprocess").unwrap(),
            ProviderKind::InProcess
        );
        assert_eq!(parse_provider("OLLAMA").unwrap(), ProviderKind::Ollama);
        assert!(parse_provider("openai").is_err());
    }

    #[test]
    fn gguf_paths_split_into_a_directory_and_filename() {
        assert_eq!(
            model_location("models/llama.gguf", None).unwrap(),
            ModelLocation::Gguf {
                model_id: "models".into(),
                files: vec!["llama.gguf".into()],
            }
        );
        assert_eq!(
            model_location("org/model", Some("q4.gguf")).unwrap(),
            ModelLocation::Gguf {
                model_id: "org/model".into(),
                files: vec!["q4.gguf".into()],
            }
        );
        assert_eq!(
            model_location("Qwen/Qwen3-4B", None).unwrap(),
            ModelLocation::Auto("Qwen/Qwen3-4B".into())
        );
    }

    #[test]
    fn isq_is_only_for_a_full_inprocess_model() {
        let bits = Some(IsqBits::Four);
        assert!(check_isq(ProviderKind::Ollama, bits, &ModelLocation::Auto("m".into())).is_err());
        assert!(
            check_isq(
                ProviderKind::InProcess,
                bits,
                &ModelLocation::Gguf {
                    model_id: "models".into(),
                    files: vec!["model.gguf".into()],
                },
            )
            .is_err()
        );
        assert!(
            check_isq(
                ProviderKind::InProcess,
                bits,
                &ModelLocation::Auto("org/model".into())
            )
            .is_ok()
        );
        assert!(
            check_isq(
                ProviderKind::InProcess,
                None,
                &ModelLocation::Gguf {
                    model_id: "models".into(),
                    files: vec!["model.gguf".into()],
                },
            )
            .is_ok()
        );
    }
}
