use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const APP_NAME: &str = "speake-rs";

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn config_home() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".config"))
}

pub fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".local/share"))
}

pub fn runtime_home() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn app_config_dir() -> PathBuf {
    config_home().join(APP_NAME)
}

pub fn app_data_dir() -> PathBuf {
    data_home().join(APP_NAME)
}

pub fn app_runtime_dir() -> PathBuf {
    runtime_home().join(APP_NAME)
}

pub fn config_path() -> PathBuf {
    app_config_dir().join("config.toml")
}

pub fn socket_path() -> PathBuf {
    app_runtime_dir().join("daemon.sock")
}

pub fn pid_path() -> PathBuf {
    app_runtime_dir().join("daemon.pid")
}

pub fn profiles_dir() -> PathBuf {
    app_data_dir().join("voices")
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory: {}", parent.display()))
}

pub fn ensure_runtime_dir() -> Result<PathBuf> {
    let dir = app_runtime_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create runtime dir: {}", dir.display()))?;
    Ok(dir)
}

pub fn ensure_profiles_dir() -> Result<PathBuf> {
    let dir = profiles_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create profiles dir: {}", dir.display()))?;
    Ok(dir)
}
