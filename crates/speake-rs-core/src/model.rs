use std::fmt;
use std::str::FromStr;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const CUSTOM_VOICE_MODEL_ID: &str = "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice";
pub const BASE_MODEL_ID: &str = "Qwen/Qwen3-TTS-12Hz-1.7B-Base";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelVariant {
    Base,
    CustomVoice,
}

impl Default for ModelVariant {
    fn default() -> Self {
        Self::CustomVoice
    }
}

impl ModelVariant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::CustomVoice => "custom-voice",
        }
    }

    pub fn model_id(self) -> &'static str {
        match self {
            Self::Base => BASE_MODEL_ID,
            Self::CustomVoice => CUSTOM_VOICE_MODEL_ID,
        }
    }
}

impl fmt::Display for ModelVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ModelVariant {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "base" => Ok(Self::Base),
            "custom-voice" | "custom_voice" | "customvoice" => Ok(Self::CustomVoice),
            _ => anyhow::bail!(
                "unsupported model variant '{input}', expected one of: base, custom-voice"
            ),
        }
    }
}
