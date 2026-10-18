//! Small adapter dispatcher; each provider owns its network and accounting formats.
pub use super::{
    claude::claude_limits,
    codex::codex_limits,
    cursor::{cursor_limits, parse_cursor_events},
};
use crate::model::{LimitWindow, Provider, UsageRecord};
use anyhow::Result;
use reqwest::blocking::Client;
use std::{path::PathBuf, time::Duration};
pub struct LiveData {
    pub limits: Vec<LimitWindow>,
    pub plan: Option<String>,
    pub records: Vec<UsageRecord>,
}
pub(super) fn fingerprint(provider: Provider, homes: &[PathBuf]) -> Result<Option<String>> {
    match provider {
        Provider::Kiro => super::kiro::fingerprint(homes),
        Provider::Antigravity => super::antigravity::fingerprint(homes),
        Provider::OpenCode => super::opencode::fingerprint(homes),
        _ => Ok(None),
    }
}
pub fn fetch(provider: Provider, homes: &[PathBuf], history: bool) -> Result<LiveData> {
    let client = Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("TokenDock/", env!("CARGO_PKG_VERSION")))
        .build()?;
    match provider {
        Provider::Codex => super::codex::fetch(&client, homes),
        Provider::Claude => super::claude::fetch(&client, homes),
        Provider::Cursor => super::cursor::fetch(&client, homes, history),
        Provider::Kiro => super::kiro::fetch(&client, homes),
        Provider::Antigravity => super::antigravity::fetch(&client, homes),
        Provider::OpenCode => super::opencode::fetch(&client, homes),
    }
}
