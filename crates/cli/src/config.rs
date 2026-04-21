//! Minimal CLI config reader.
//!
//! Priority (highest first):
//!   1. Environment variables: `DECIPHER_API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`
//!   2. `~/.decipher/config.json`
//!   3. Built-in defaults
//!
//! Only ports what is needed to run `AgentLoop`. Comprehensive config migration
//! (from `lib/config.js`) is R3 work.

use decipher_policy::PolicyMode;
use serde::Deserialize;

/// Which provider protocol to use for LLM calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderType {
    /// Anthropic /v1/messages protocol (x-api-key header).
    Anthropic,
    /// OpenAI /v1/chat/completions protocol (Bearer auth).
    /// Also used for ZhiPu, Groq, Mistral, Together, Deepseek, vLLM, etc.
    OpenAi,
}

/// Runtime configuration for the CLI.
#[derive(Debug, Clone)]
pub struct CliConfig {
    pub api_key: String,
    pub model: String,
    /// Optional base URL override (e.g. for local proxies or alternative providers).
    pub base_url: Option<String>,
    /// Working directory — defaults to `$PWD`.
    pub workspace: String,
    pub policy_mode: PolicyMode,
    /// If true, start in plan mode (generate plan before executing).
    pub plan_mode_flag: bool,
    /// Which provider protocol to use.
    pub provider_type: ProviderType,
}

#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    policy_mode: Option<String>,
    /// Provider type: "anthropic", "openai", or "auto" (default).
    provider: Option<String>,
}

impl CliConfig {
    /// Load config from environment variables and `~/.decipher/config.json`.
    pub fn load() -> Self {
        let file: ConfigFile = read_config_file().unwrap_or_default();

        let api_key = std::env::var("DECIPHER_API_KEY")
            .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()
            .or(file.api_key)
            .unwrap_or_default();

        let model = std::env::var("DECIPHER_MODEL")
            .ok()
            .or(file.model)
            .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

        let base_url = std::env::var("DECIPHER_BASE_URL").ok().or(file.base_url);

        let policy_mode = match file.policy_mode.as_deref() {
            Some("read-only") => PolicyMode::ReadOnly,
            Some("granular") => PolicyMode::Granular,
            Some("full-access") => PolicyMode::FullAccess,
            _ => PolicyMode::Auto,
        };

        let workspace = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // --plan flag from CLI args.
        let plan_mode_flag = std::env::args().any(|a| a == "--plan");

        // Provider detection: explicit > env > auto-detect from base_url/model.
        let provider_type = std::env::var("DECIPHER_PROVIDER")
            .ok()
            .or(file.provider)
            .map(|s| match s.to_lowercase().as_str() {
                "anthropic" => ProviderType::Anthropic,
                "openai" => ProviderType::OpenAi,
                _ => auto_detect_provider(base_url.as_deref(), &model),
            })
            .unwrap_or_else(|| auto_detect_provider(base_url.as_deref(), &model));

        Self { api_key, model, base_url, workspace, policy_mode, plan_mode_flag, provider_type }
    }
}

/// Return the DeCIpher home directory (`$DECIPHER_CONFIG_DIR` or `~/.decipher`).
pub fn decipher_home() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("DECIPHER_CONFIG_DIR") {
        return dir.into();
    }
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".decipher"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".decipher"))
}

/// Read `~/.decipher/config.json` if it exists.
fn read_config_file() -> Option<ConfigFile> {
    let path = decipher_home().join("config.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Auto-detect provider from base_url and model name.
///
/// Default → Anthropic (native Claude). Custom base_url → OpenAI-compatible
/// unless it's `api.anthropic.com`.
pub fn auto_detect_provider(base_url: Option<&str>, model: &str) -> ProviderType {
    // Explicit Anthropic endpoint or claude model with default endpoint.
    match base_url {
        None => {
            // No custom URL. Anthropic if it looks like a Claude model, OpenAI otherwise.
            if model.starts_with("claude-") {
                ProviderType::Anthropic
            } else if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
                ProviderType::OpenAi
            } else {
                // Unknown model, no base_url — default Anthropic (the original behavior).
                ProviderType::Anthropic
            }
        }
        Some(url) => {
            if url.contains("anthropic.com") {
                ProviderType::Anthropic
            } else {
                // Any custom base URL → OpenAI-compatible protocol.
                ProviderType::OpenAi
            }
        }
    }
}
