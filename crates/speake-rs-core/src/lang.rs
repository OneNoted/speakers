use anyhow::Result;
use qwen3_tts::{Language, Speaker};

pub const DEFAULT_LANGUAGE: &str = "en";
pub const DEFAULT_PRESET_VOICE: &str = "ryan";

/// Parse speech-dispatcher language codes. Region suffixes are ignored.
pub fn parse_language(code: &str) -> Result<Language> {
    let base = code.split('-').next().unwrap_or(code);
    base.parse::<Language>()
        .map_err(|e| anyhow::anyhow!("unknown language '{code}': {e}"))
}

/// Parse a preset Qwen3 voice name.
pub fn parse_speaker(name: &str) -> Result<Speaker> {
    name.parse::<Speaker>()
        .map_err(|e| anyhow::anyhow!("unknown preset voice '{name}': {e}"))
}
