use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use candle_core::Device;
use qwen3_tts::VoiceClonePrompt;
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    Icl,
    Xvector,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub version: u32,
    pub name: String,
    pub mode: ProfileMode,
    pub created_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_text_ids: Option<Vec<u32>>,
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("profile name cannot be empty");
    }

    let ok = name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !ok {
        anyhow::bail!("profile name can only contain [A-Za-z0-9_-]");
    }

    Ok(())
}

fn profile_dir(name: &str) -> PathBuf {
    paths::profiles_dir().join(name)
}

pub fn save_profile(name: &str, prompt: &VoiceClonePrompt) -> Result<()> {
    validate_profile_name(name)?;

    let dir = profile_dir(name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create profile dir: {}", dir.display()))?;

    let mut tensors = HashMap::new();
    tensors.insert(
        "speaker_embedding".to_string(),
        prompt.speaker_embedding.clone(),
    );

    let (mode, ref_text_ids) =
        if let (Some(codes), Some(ids)) = (&prompt.ref_codes, &prompt.ref_text_ids) {
            tensors.insert("ref_codes".to_string(), codes.clone());
            (ProfileMode::Icl, Some(ids.clone()))
        } else {
            (ProfileMode::Xvector, None)
        };

    let tensor_path = dir.join("tensors.safetensors");
    candle_core::safetensors::save(&tensors, &tensor_path)
        .with_context(|| format!("failed to write tensors: {}", tensor_path.display()))?;

    let created_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = ProfileMeta {
        version: 1,
        name: name.to_string(),
        mode,
        created_at_unix,
        ref_text_ids,
    };

    let meta_path = dir.join("profile.json");
    let meta_body =
        serde_json::to_string_pretty(&meta).context("failed to serialize profile meta")?;
    std::fs::write(&meta_path, meta_body)
        .with_context(|| format!("failed to write profile meta: {}", meta_path.display()))?;

    Ok(())
}

pub fn load_profile(name: &str, device: &Device) -> Result<VoiceClonePrompt> {
    validate_profile_name(name)?;

    let dir = profile_dir(name);
    anyhow::ensure!(dir.is_dir(), "profile '{name}' does not exist");

    let meta = read_profile_meta(name)?;

    let tensor_path = dir.join("tensors.safetensors");
    let tensors = candle_core::safetensors::load(&tensor_path, device)
        .with_context(|| format!("failed to read tensors: {}", tensor_path.display()))?;

    let speaker_embedding = tensors
        .get("speaker_embedding")
        .context("missing speaker_embedding in profile tensors")?
        .clone();

    let (ref_codes, ref_text_ids) = if meta.mode == ProfileMode::Icl {
        let codes = tensors
            .get("ref_codes")
            .context("missing ref_codes in ICL profile tensors")?
            .clone();
        (Some(codes), meta.ref_text_ids)
    } else {
        (None, None)
    };

    Ok(VoiceClonePrompt {
        speaker_embedding,
        ref_codes,
        ref_text_ids,
    })
}

pub fn read_profile_meta(name: &str) -> Result<ProfileMeta> {
    validate_profile_name(name)?;

    let meta_path = profile_dir(name).join("profile.json");
    let body = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("failed to read profile meta: {}", meta_path.display()))?;
    let meta: ProfileMeta = serde_json::from_str(&body)
        .with_context(|| format!("failed to parse profile meta: {}", meta_path.display()))?;
    Ok(meta)
}

pub fn list_profiles() -> Result<Vec<String>> {
    let dir = paths::ensure_profiles_dir()?;
    let mut names = Vec::new();

    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read profiles dir: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if !path.join("profile.json").exists() {
            continue;
        }

        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }

    names.sort();
    Ok(names)
}
