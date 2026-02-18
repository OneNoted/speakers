use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::ModelVariant;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceSelection {
    Preset { name: String },
    Profile { name: String },
}

impl VoiceSelection {
    pub fn preset(name: impl Into<String>) -> Self {
        Self::Preset { name: name.into() }
    }

    pub fn profile(name: impl Into<String>) -> Self {
        Self::Profile { name: name.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonRequest {
    Health,
    Shutdown,
    Synthesize {
        text: String,
        language: String,
        output: PathBuf,
        voice: VoiceSelection,
        #[serde(skip_serializing_if = "Option::is_none")]
        rate: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pitch: Option<i32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthData {
    pub pid: u32,
    pub model: ModelVariant,
    pub device: String,
    pub socket: PathBuf,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisData {
    pub output: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ResponseData {
    Ack,
    Health(HealthData),
    Synthesis(SynthesisData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_details: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
}

impl DaemonResponse {
    pub fn ok(data: Option<ResponseData>) -> Self {
        Self {
            ok: true,
            error_code: None,
            error_message: None,
            error_details: None,
            data,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::error_with_details(code, message, Vec::<String>::new())
    }

    pub fn error_with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Vec<String>,
    ) -> Self {
        Self {
            ok: false,
            error_code: Some(code.into()),
            error_message: Some(message.into()),
            error_details: (!details.is_empty()).then_some(details),
            data: None,
        }
    }
}
