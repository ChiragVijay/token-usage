use crate::model::Provider;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

pub const DEFAULT_REFRESH_SECONDS: u64 = 1800;
pub const REFRESH_INTERVALS: [u64; 4] = [300, 600, 1800, 3600];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub online: bool,
    #[serde(deserialize_with = "enabled")]
    pub enabled: [bool; Provider::COUNT],
    pub dark: bool,
    pub privacy: bool,
    pub refresh_seconds: u64,
    /// Version postponed with Delay. The next launch asks again.
    pub update_delayed: Option<String>,
    #[serde(deserialize_with = "custom_homes")]
    pub custom_homes: [Vec<PathBuf>; Provider::COUNT],
    pub cursor_imports: Vec<PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            online: false,
            enabled: [true; Provider::COUNT],
            dark: false,
            privacy: false,
            refresh_seconds: DEFAULT_REFRESH_SECONDS,
            update_delayed: None,
            custom_homes: Default::default(),
            cursor_imports: vec![],
        }
    }
}
fn enabled<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[bool; Provider::COUNT], D::Error> {
    let values = Vec::<bool>::deserialize(deserializer)?;
    let mut result = [true; Provider::COUNT];
    for (slot, value) in result.iter_mut().zip(values) {
        *slot = value;
    }
    Ok(result)
}
fn custom_homes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[Vec<PathBuf>; Provider::COUNT], D::Error> {
    let values = Vec::<Vec<PathBuf>>::deserialize(deserializer)?;
    let mut result = std::array::from_fn(|_| vec![]);
    for (slot, value) in result.iter_mut().zip(values) {
        *slot = value;
    }
    Ok(result)
}
pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("TOKEN_DOCK_DATA_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    dirs::data_local_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
        .context("No per-user application data directory is available")
        .map(|base| base.join("token-dock"))
}
pub(crate) fn private_data_dir() -> Result<PathBuf> {
    let path = data_dir()?;
    crate::storage::private_directory(&path)?;
    Ok(path)
}
impl Config {
    pub fn load() -> Result<Self> {
        let path = data_dir()?.join("settings.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut config: Self = serde_json::from_slice(&crate::input::read(
            fs::File::open(&path)?,
            crate::input::AUTH_BYTES,
        )?)
        .context("Settings could not be read")?;
        if !REFRESH_INTERVALS.contains(&config.refresh_seconds) {
            config.refresh_seconds = DEFAULT_REFRESH_SECONDS;
        }
        Ok(config)
    }
    pub fn save(&self) -> Result<()> {
        let path = private_data_dir()?.join("settings.json");
        crate::storage::atomic_write(&path, &serde_json::to_vec_pretty(self)?)
            .context("Settings could not be saved")
    }
}
