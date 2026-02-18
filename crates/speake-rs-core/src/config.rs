use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::lang::{DEFAULT_LANGUAGE, DEFAULT_PRESET_VOICE};
use crate::model::ModelVariant;
use crate::paths;
use crate::protocol::VoiceSelection;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub speech_dispatcher: SpeechDispatcherConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            speech_dispatcher: SpeechDispatcherConfig::default(),
        }
    }
}

impl Config {
    pub fn load_or_create() -> Result<Self> {
        let path = paths::config_path();
        if !path.exists() {
            let cfg = Self::default();
            cfg.save()?;
            return Ok(cfg);
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        toml::from_str::<Self>(&raw)
            .with_context(|| format!("failed to parse config: {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_path();
        paths::ensure_parent(&path)?;
        let body = toml::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(&path, body)
            .with_context(|| format!("failed to write config: {}", path.display()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub model: ModelVariant,
    pub socket_path: Option<PathBuf>,
    pub request_timeout_ms: u64,
    pub synthesis_timeout_ms: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            model: ModelVariant::CustomVoice,
            socket_path: None,
            request_timeout_ms: 60_000,
            synthesis_timeout_ms: 90_000,
        }
    }
}

impl DaemonConfig {
    pub fn resolved_socket_path(&self) -> PathBuf {
        self.socket_path.clone().unwrap_or_else(paths::socket_path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeechDispatcherConfig {
    pub playback_command: Option<String>,
    pub default_language: String,
    pub default_symbolic_voice: String,
    pub allow_icl: bool,
    pub fallback_profile: Option<String>,
    pub voice_map: BTreeMap<String, String>,
}

impl Default for SpeechDispatcherConfig {
    fn default() -> Self {
        let mut voice_map = BTreeMap::new();
        voice_map.insert("MALE1".to_string(), "preset:ryan".to_string());
        voice_map.insert("MALE2".to_string(), "preset:aiden".to_string());
        voice_map.insert("MALE3".to_string(), "preset:dylan".to_string());
        voice_map.insert("FEMALE1".to_string(), "preset:vivian".to_string());
        voice_map.insert("FEMALE2".to_string(), "preset:serena".to_string());
        voice_map.insert("FEMALE3".to_string(), "preset:sohee".to_string());
        voice_map.insert("CHILD_MALE".to_string(), "preset:eric".to_string());
        voice_map.insert("CHILD_FEMALE".to_string(), "preset:ono_anna".to_string());

        Self {
            playback_command: None,
            default_language: DEFAULT_LANGUAGE.to_string(),
            default_symbolic_voice: "MALE1".to_string(),
            allow_icl: false,
            fallback_profile: None,
            voice_map,
        }
    }
}

impl SpeechDispatcherConfig {
    pub fn resolve_voice_selection(&self, symbolic_voice: Option<&str>) -> Result<VoiceSelection> {
        let key = symbolic_voice
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.default_symbolic_voice)
            .to_ascii_uppercase();

        let binding = self
            .voice_map
            .get(&key)
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_PRESET_VOICE);

        parse_voice_binding(binding)
    }

    pub fn fallback_voice_selection(&self) -> Option<VoiceSelection> {
        self.fallback_profile
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(VoiceSelection::profile)
    }
}

pub fn parse_voice_binding(binding: &str) -> Result<VoiceSelection> {
    let trimmed = binding.trim();
    if let Some(name) = trimmed.strip_prefix("preset:") {
        return Ok(VoiceSelection::preset(name.trim().to_string()));
    }

    if let Some(name) = trimmed.strip_prefix("profile:") {
        return Ok(VoiceSelection::profile(name.trim().to_string()));
    }

    // Backward-compatible shorthand: bare voice means preset.
    Ok(VoiceSelection::preset(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_profile_binding() {
        let resolved = parse_voice_binding("profile:my_voice").expect("parse");
        match resolved {
            VoiceSelection::Profile { name } => assert_eq!(name, "my_voice"),
            _ => panic!("unexpected voice kind"),
        }
    }

    #[test]
    fn map_preset_binding() {
        let resolved = parse_voice_binding("preset:ryan").expect("parse");
        match resolved {
            VoiceSelection::Preset { name } => assert_eq!(name, "ryan"),
            _ => panic!("unexpected voice kind"),
        }
    }

    #[test]
    fn fallback_profile_maps_to_profile_voice() {
        let cfg = SpeechDispatcherConfig {
            fallback_profile: Some("backup_xvec".to_string()),
            ..SpeechDispatcherConfig::default()
        };

        let selection = cfg.fallback_voice_selection().expect("fallback voice");
        match selection {
            VoiceSelection::Profile { name } => assert_eq!(name, "backup_xvec"),
            _ => panic!("unexpected fallback voice kind"),
        }
    }

    #[test]
    fn empty_fallback_profile_is_ignored() {
        let cfg = SpeechDispatcherConfig {
            fallback_profile: Some("   ".to_string()),
            ..SpeechDispatcherConfig::default()
        };

        assert!(cfg.fallback_voice_selection().is_none());
    }
}
