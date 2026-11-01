//! One choice per account: on or off, and at most one extra path.
//!
//! The saved settings can still hold several custom folders. [`Connections::open`]
//! paints one hint and writes nothing. The first successful [`Connections::choose`]
//! stores that reduced set.

use crate::{
    analytics,
    config::Config,
    model::{Provider, ProviderStatus, Snapshot},
    providers::{self, history},
};
use std::path::{Path, PathBuf};

/// Config (inclusion, custom homes, the Cursor export, privacy, online)
/// stays behind the implementor.
pub trait Connections {
    /// Paint the saved set against the latest snapshot. Writes nothing.
    fn open(&self, snapshot: &Snapshot) -> View;

    /// Save `choices` as the reading set, then paint it.
    /// On `Err`, nothing is written.
    fn choose(&mut self, snapshot: &Snapshot, choices: Choices) -> Result<View, Reject>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choices {
    pub chatgpt: Choice,
    pub claude_code: Choice,
    pub cursor: Choice,
    pub kiro: Choice,
    pub antigravity: Choice,
    pub opencode: Choice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub on: bool,
    /// Extra place to look. `None` means discovered homes only.
    pub hint: Option<Hint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hint {
    /// Data directory. Stored even when the path does not exist yet.
    Folder(PathBuf),
    /// Cursor usage export. Legal only on `choices.cursor`.
    Export(PathBuf),
}

impl Hint {
    pub fn path(&self) -> &Path {
        match self {
            Self::Folder(path) | Self::Export(path) => path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub chatgpt: Row,
    pub claude_code: Row,
    pub cursor: Row,
    pub kiro: Row,
    pub antigravity: Row,
    pub opencode: Row,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// `codex`, `claude`, `cursor`, `kiro`, `antigravity`, `opencode`.
    pub key: &'static str,
    /// ChatGPT, Claude Code, Cursor, Kiro, Antigravity, OpenCode.
    pub title: &'static str,
    pub on: bool,
    /// Round-trip into the next `Choice`. The path inside is not display text.
    pub hint: Option<Hint>,
    /// Privacy-safe label for `hint`. `None` when the row is off or has no hint.
    pub hint_label: Option<String>,
    /// Discovered homes that exist, privacy-safe. Empty when the row is off.
    pub found: Vec<String>,
    /// Muted line under the title. `None` when the row is off or there is nothing to say.
    pub caption: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reject {
    /// Ready to show. The saved set is unchanged.
    pub message: String,
}

impl From<&View> for Choices {
    fn from(view: &View) -> Self {
        Self {
            chatgpt: Choice::from_row(&view.chatgpt),
            claude_code: Choice::from_row(&view.claude_code),
            cursor: Choice::from_row(&view.cursor),
            kiro: Choice::from_row(&view.kiro),
            antigravity: Choice::from_row(&view.antigravity),
            opencode: Choice::from_row(&view.opencode),
        }
    }
}

impl Choice {
    fn from_row(row: &Row) -> Self {
        Self {
            on: row.on,
            hint: row.hint.clone(),
        }
    }
}

impl Choices {
    pub fn get(&self, provider: Provider) -> &Choice {
        match provider {
            Provider::Codex => &self.chatgpt,
            Provider::Claude => &self.claude_code,
            Provider::Cursor => &self.cursor,
            Provider::Kiro => &self.kiro,
            Provider::Antigravity => &self.antigravity,
            Provider::OpenCode => &self.opencode,
        }
    }

    pub fn get_mut(&mut self, provider: Provider) -> &mut Choice {
        match provider {
            Provider::Codex => &mut self.chatgpt,
            Provider::Claude => &mut self.claude_code,
            Provider::Cursor => &mut self.cursor,
            Provider::Kiro => &mut self.kiro,
            Provider::Antigravity => &mut self.antigravity,
            Provider::OpenCode => &mut self.opencode,
        }
    }
}

impl View {
    pub fn row(&self, provider: Provider) -> &Row {
        match provider {
            Provider::Codex => &self.chatgpt,
            Provider::Claude => &self.claude_code,
            Provider::Cursor => &self.cursor,
            Provider::Kiro => &self.kiro,
            Provider::Antigravity => &self.antigravity,
            Provider::OpenCode => &self.opencode,
        }
    }

    pub fn row_mut(&mut self, provider: Provider) -> &mut Row {
        match provider {
            Provider::Codex => &mut self.chatgpt,
            Provider::Claude => &mut self.claude_code,
            Provider::Cursor => &mut self.cursor,
            Provider::Kiro => &mut self.kiro,
            Provider::Antigravity => &mut self.antigravity,
            Provider::OpenCode => &mut self.opencode,
        }
    }
}

impl Row {
    /// Reflect one edited choice before [`Connections::choose`] paints again.
    /// An off row hides its caption, found paths, and hint label. The hint stays.
    pub fn preview(&mut self, choice: &Choice, privacy: bool) {
        self.on = choice.on;
        self.hint = choice.hint.clone();
        if !self.on {
            self.caption = None;
            self.hint_label = None;
            self.found.clear();
            return;
        }
        self.hint_label = self
            .hint
            .as_ref()
            .map(|hint| path_label(hint.path(), privacy));
    }
}

/// The reading set behind the connections page.
pub struct Reading {
    config: Config,
}

impl Reading {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn into_config(self) -> Config {
        self.config
    }

    /// The on/off bit and the one hint each account would keep. Does not touch disk.
    pub fn draft(&self) -> Choices {
        choices_from(&self.config)
    }

    /// Paint captions and hints without checking which default folders exist.
    pub fn recall(&self, snapshot: &Snapshot) -> View {
        paint(&self.config, snapshot, &std::array::from_fn(|_| Vec::new()))
    }
}

impl Connections for Reading {
    fn open(&self, snapshot: &Snapshot) -> View {
        paint(&self.config, snapshot, &discovered())
    }

    fn choose(&mut self, snapshot: &Snapshot, choices: Choices) -> Result<View, Reject> {
        validate(&choices)?;
        apply(&mut self.config, &choices);
        Ok(self.open(snapshot))
    }
}

fn choices_from(config: &Config) -> Choices {
    Choices {
        chatgpt: choice_for(config, Provider::Codex),
        claude_code: choice_for(config, Provider::Claude),
        cursor: choice_for(config, Provider::Cursor),
        kiro: choice_for(config, Provider::Kiro),
        antigravity: choice_for(config, Provider::Antigravity),
        opencode: choice_for(config, Provider::OpenCode),
    }
}

fn choice_for(config: &Config, provider: Provider) -> Choice {
    Choice {
        on: config.enabled[provider.index()],
        hint: stored_hint(config, provider),
    }
}

/// Cursor keeps an export when one is saved. Otherwise the hint is the last custom folder.
fn stored_hint(config: &Config, provider: Provider) -> Option<Hint> {
    if provider == Provider::Cursor
        && let Some(path) = config.cursor_imports.last()
    {
        return Some(Hint::Export(path.clone()));
    }
    config.custom_homes[provider.index()]
        .last()
        .cloned()
        .map(Hint::Folder)
}

fn discovered() -> [Vec<PathBuf>; Provider::COUNT] {
    let bare = Config::default();
    std::array::from_fn(|index| {
        providers::homes(Provider::ALL[index], &bare)
            .into_iter()
            .filter(|path| path.exists())
            .collect()
    })
}

fn paint(config: &Config, snapshot: &Snapshot, homes: &[Vec<PathBuf>; Provider::COUNT]) -> View {
    let choices = choices_from(config);
    View {
        chatgpt: paint_row(Provider::Codex, &choices.chatgpt, config, snapshot, homes),
        claude_code: paint_row(
            Provider::Claude,
            &choices.claude_code,
            config,
            snapshot,
            homes,
        ),
        cursor: paint_row(Provider::Cursor, &choices.cursor, config, snapshot, homes),
        kiro: paint_row(Provider::Kiro, &choices.kiro, config, snapshot, homes),
        antigravity: paint_row(
            Provider::Antigravity,
            &choices.antigravity,
            config,
            snapshot,
            homes,
        ),
        opencode: paint_row(
            Provider::OpenCode,
            &choices.opencode,
            config,
            snapshot,
            homes,
        ),
    }
}

fn paint_row(
    provider: Provider,
    choice: &Choice,
    config: &Config,
    snapshot: &Snapshot,
    homes: &[Vec<PathBuf>; Provider::COUNT],
) -> Row {
    let status = snapshot
        .providers
        .iter()
        .find(|status| status.provider == provider)
        .cloned()
        .unwrap_or_else(|| ProviderStatus::empty(provider));
    Row {
        key: provider.key(),
        title: provider.account_name(),
        on: choice.on,
        hint: choice.hint.clone(),
        hint_label: if choice.on {
            choice
                .hint
                .as_ref()
                .map(|hint| path_label(hint.path(), config.privacy))
        } else {
            None
        },
        found: if choice.on {
            found_labels(
                &homes[provider.index()],
                choice.hint.as_ref(),
                config.privacy,
            )
        } else {
            Vec::new()
        },
        caption: caption_line(config, &status, choice),
    }
}

fn found_labels(homes: &[PathBuf], hint: Option<&Hint>, privacy: bool) -> Vec<String> {
    let skip = hint.map(|hint| path_key(hint.path()));
    homes
        .iter()
        .filter(|path| Some(path_key(path)) != skip)
        .map(|path| path_label(path, privacy))
        .collect()
}

fn caption_line(config: &Config, status: &ProviderStatus, choice: &Choice) -> Option<String> {
    if !choice.on {
        return None;
    }
    let mut parts = Vec::new();
    if status.provider == Provider::Cursor && matches!(choice.hint, Some(Hint::Export(_))) {
        parts.push("CSV".into());
    } else if status.provider != Provider::Cursor && status.sources > 0 {
        parts.push(if status.sources == 1 {
            "1 file".into()
        } else {
            format!("{} files", status.sources)
        });
    }
    if let Some(allowance) = allowance(config, status) {
        let allowance = plain_text(&allowance);
        if !allowance.is_empty() {
            parts.push(allowance);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(analytics::separator()))
    }
}

fn allowance(config: &Config, status: &ProviderStatus) -> Option<String> {
    if !config.online {
        return Some(if status.checked_at.is_some() {
            "Cached".into()
        } else {
            "Paused".into()
        });
    }
    if !status.stale && status.checked_at.is_some() {
        return Some(
            status
                .plan
                .as_deref()
                .map(plan_name)
                .unwrap_or_else(|| "Live".into()),
        );
    }
    if reported_error(&status.detail) {
        return Some(status.detail.clone());
    }
    if status.checked_at.is_some() {
        return Some("Cached".into());
    }
    if status.detected {
        return Some("No sign-in".into());
    }
    None
}

fn reported_error(detail: &str) -> bool {
    let sep = analytics::separator();
    let known = [
        "Paused in Connections".to_string(),
        format!("Account history{sep}across your machines"),
        "Local history + live account limits".to_string(),
        "Live account limits".to_string(),
        "Cached Cursor account history".to_string(),
        format!("Detected on this device{sep}no local usage yet"),
        "Import a usage CSV or enable account checks".to_string(),
        "Connect a source to see usage".to_string(),
    ];
    if detail.is_empty()
        || known.iter().any(|item| item == detail)
        || detail.ends_with(&format!("history files{sep}local"))
    {
        return false;
    }
    !(detail.starts_with("Use ") && detail.contains("to create local history"))
}

fn plan_name(plan: &str) -> String {
    plan.split(['_', '-'])
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| format!("{}{}", first.to_uppercase(), chars.as_str()))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate(choices: &Choices) -> Result<(), Reject> {
    for provider in Provider::ALL {
        let Some(Hint::Export(path)) = &choices.get(provider).hint else {
            continue;
        };
        if provider != Provider::Cursor {
            return Err(Reject {
                message: "A usage export is only for Cursor".into(),
            });
        }
        if let Err(error) = history::read_cursor_csv(path) {
            return Err(Reject {
                message: reject_text(error.to_string()),
            });
        }
    }
    Ok(())
}

fn apply(config: &mut Config, choices: &Choices) {
    for provider in Provider::ALL {
        let choice = choices.get(provider);
        let index = provider.index();
        config.enabled[index] = choice.on;
        config.custom_homes[index] = match &choice.hint {
            Some(Hint::Folder(path)) => vec![path.clone()],
            _ => Vec::new(),
        };
    }
    config.cursor_imports = match &choices.cursor.hint {
        Some(Hint::Export(path)) => vec![path.clone()],
        _ => Vec::new(),
    };
}

/// Label for a hint or a discovered home. Privacy draws `Hidden`. An empty path draws `-`.
pub fn path_label(path: &Path, privacy: bool) -> String {
    if privacy {
        return "Hidden".into();
    }
    let text = plain_text(&display_path(path));
    if text.is_empty() {
        analytics::empty_value().into()
    } else {
        text
    }
}

fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text
    }
}

fn path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            '\u{2014}' | '\u{2013}' | '\u{2212}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2022}' | '\u{00b7}' | '\u{2219}' | '\u{2027}' => out.push('|'),
            '\u{2190}'..='\u{21ff}'
            | '\u{2794}'
            | '\u{279c}'
            | '\u{27a1}'
            | '\u{27a4}'
            | '\u{2b05}'..='\u{2b0d}'
            | '\u{2900}'..='\u{297f}' => out.push('-'),
            _ => out.push(ch),
        }
    }
    out
}

fn reject_text(text: String) -> String {
    let text = plain_text(&text);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        "The Cursor usage export could not be read".into()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderStatus;
    use chrono::Utc;

    fn forbidden(text: &str) -> bool {
        text.chars().any(|ch| {
            matches!(
                ch,
                '\u{2014}' | '\u{2013}' | '\u{2026}' | '\u{2022}' | '\u{00b7}' | '\u{2190}'
                    ..='\u{21ff}' | '\u{2794}' | '\u{27a1}'
            )
        })
    }

    const CSV: &str =
        "Date,Model,Input Tokens,Output Tokens,Cost\n2026-10-02T13:00:00Z,gpt,10,5,$0.50\n";

    fn reading(config: Config) -> Reading {
        Reading::new(config)
    }

    fn fresh(provider: Provider, plan: Option<&str>) -> ProviderStatus {
        let mut status = ProviderStatus::empty(provider);
        status.stale = false;
        status.checked_at = Some(Utc::now());
        status.plan = plan.map(str::to_string);
        status.connected = true;
        status
    }

    fn assert_authored(text: &str) {
        assert!(text.is_ascii(), "{text}");
        assert!(!forbidden(text), "{text}");
        assert_ne!(text, "\u{2014}");
    }

    #[test]
    fn six_accounts_stay_on_the_page() {
        let view = reading(Config::default()).open(&Snapshot::default());
        let rows = [
            view.row(Provider::Codex),
            view.row(Provider::Claude),
            view.row(Provider::Cursor),
            view.row(Provider::Kiro),
            view.row(Provider::Antigravity),
            view.row(Provider::OpenCode),
        ];
        let titles = [
            "ChatGPT",
            "Claude Code",
            "Cursor",
            "Kiro",
            "Antigravity",
            "OpenCode",
        ];
        for (row, title) in rows.iter().zip(titles) {
            assert_eq!(row.title, title);
            assert!(row.on);
            assert!(row.hint.is_none());
            assert!(row.hint_label.is_none());
            assert!(row.key.is_ascii());
            row.caption
                .as_deref()
                .inspect(|caption| assert_authored(caption));
            for found in &row.found {
                assert!(!forbidden(found), "{found}");
            }
        }
    }

    #[test]
    fn turning_a_row_off_hides_the_caption_and_keeps_the_hint() {
        let mut config = Config::default();
        let folder = PathBuf::from("kiro-home");
        let older = PathBuf::from("kiro-old");
        config.custom_homes[Provider::Kiro.index()] = vec![older.clone(), folder.clone()];
        config.online = false;
        let mut app = reading(config);
        let view = app.open(&Snapshot::default());
        assert_eq!(view.kiro.hint, Some(Hint::Folder(folder.clone())));
        let label = path_label(&folder, false);
        assert_eq!(view.kiro.hint_label.as_deref(), Some(label.as_str()));
        assert_eq!(view.kiro.caption.as_deref(), Some("Paused"));
        assert_eq!(
            app.config().custom_homes[Provider::Kiro.index()],
            vec![older.clone(), folder.clone()]
        );

        let mut choices = Choices::from(&view);
        choices.kiro.on = false;
        let view = app.choose(&Snapshot::default(), choices).unwrap();
        assert!(!view.kiro.on);
        assert!(view.kiro.caption.is_none());
        assert!(view.kiro.found.is_empty());
        assert!(view.kiro.hint_label.is_none());
        assert_eq!(view.kiro.hint, Some(Hint::Folder(folder.clone())));
        assert_eq!(
            app.config().custom_homes[Provider::Kiro.index()],
            vec![folder.clone()]
        );

        let mut choices = Choices::from(&view);
        choices.kiro.on = true;
        let view = app.choose(&Snapshot::default(), choices).unwrap();
        assert!(view.kiro.on);
        assert_eq!(view.kiro.hint, Some(Hint::Folder(folder.clone())));
        assert!(view.kiro.hint_label.is_some());
        assert_eq!(view.kiro.caption.as_deref(), Some("Paused"));
    }

    #[test]
    fn first_save_keeps_one_hint_and_lets_a_cursor_export_win() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("usage.csv");
        std::fs::write(&csv, CSV).unwrap();
        let older = dir.path().join("older.csv");
        std::fs::write(&older, CSV).unwrap();
        let mut config = Config::default();
        config.custom_homes[Provider::Codex.index()] =
            vec![PathBuf::from("codex-a"), PathBuf::from("codex-b")];
        config.custom_homes[Provider::Cursor.index()] =
            vec![PathBuf::from("cursor-a"), PathBuf::from("cursor-b")];
        config.cursor_imports = vec![older.clone(), csv.clone()];
        config.show_hidden[Provider::Kiro.index()] = true;
        let online = config.online;
        let mut app = reading(config);
        let snapshot = Snapshot::default();
        let view = app.open(&snapshot);
        assert_eq!(
            view.chatgpt.hint,
            Some(Hint::Folder(PathBuf::from("codex-b")))
        );
        assert_eq!(view.cursor.hint, Some(Hint::Export(csv.clone())));
        assert_eq!(app.config().cursor_imports.len(), 2);
        assert_eq!(app.config().custom_homes[Provider::Cursor.index()].len(), 2);

        let view = app.choose(&snapshot, Choices::from(&view)).unwrap();
        assert_eq!(
            app.config().custom_homes[Provider::Codex.index()],
            vec![PathBuf::from("codex-b")]
        );
        assert!(app.config().custom_homes[Provider::Cursor.index()].is_empty());
        assert_eq!(app.config().cursor_imports, vec![csv.clone()]);
        assert!(app.config().show_hidden[Provider::Kiro.index()]);
        assert_eq!(app.config().online, online);
        assert_eq!(view.cursor.hint, Some(Hint::Export(csv)));
        assert!(view.cursor.caption.as_deref().unwrap().starts_with("CSV"));
        assert!(!view.cursor.caption.as_deref().unwrap().contains("event"));
    }

    #[test]
    fn a_cursor_folder_is_kept_when_there_is_no_export() {
        let mut config = Config::default();
        config.custom_homes[Provider::Cursor.index()] =
            vec![PathBuf::from("cursor-a"), PathBuf::from("cursor-b")];
        let mut app = reading(config);
        let choices = app.draft();
        let view = app.choose(&Snapshot::default(), choices).unwrap();
        assert_eq!(
            view.cursor.hint,
            Some(Hint::Folder(PathBuf::from("cursor-b")))
        );
        assert_eq!(
            app.config().custom_homes[Provider::Cursor.index()],
            vec![PathBuf::from("cursor-b")]
        );
        assert!(app.config().cursor_imports.is_empty());
        assert!(!view.cursor.caption.as_deref().unwrap_or("").contains("CSV"));
    }

    #[test]
    fn an_export_on_another_account_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("usage.csv");
        std::fs::write(&csv, CSV).unwrap();
        let mut config = Config::default();
        config.custom_homes[Provider::Claude.index()] = vec![PathBuf::from("claude-home")];
        let saved = config.clone();
        let mut app = reading(config);
        let mut choices = app.draft();
        choices.claude_code.hint = Some(Hint::Export(csv));
        let error = app.choose(&Snapshot::default(), choices).unwrap_err();
        assert_eq!(error.message, "A usage export is only for Cursor");
        assert_authored(&error.message);
        assert_eq!(app.config().custom_homes, saved.custom_homes);
        assert_eq!(app.config().enabled, saved.enabled);
        assert_eq!(app.config().cursor_imports, saved.cursor_imports);
    }

    #[test]
    fn an_unreadable_export_leaves_the_saved_set_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("notes.txt");
        std::fs::write(&csv, "not an export\n").unwrap();
        let mut config = Config::default();
        config.custom_homes[Provider::Cursor.index()] = vec![PathBuf::from("cursor-home")];
        let saved = config.clone();
        let mut app = reading(config);
        let mut choices = app.draft();
        choices.cursor.hint = Some(Hint::Export(csv));
        let error = app.choose(&Snapshot::default(), choices).unwrap_err();
        assert!(!error.message.is_empty());
        assert_authored(&error.message);
        assert!(!error.message.contains("event"));
        assert_eq!(app.config().custom_homes, saved.custom_homes);
        assert!(app.config().cursor_imports.is_empty());
    }

    #[test]
    fn captions_keep_the_short_form() {
        let mut snapshot = Snapshot::default();
        snapshot.providers[Provider::Claude.index()] = fresh(Provider::Claude, None);
        snapshot.providers[Provider::Claude.index()].sources = 1;
        snapshot.providers[Provider::Codex.index()] = fresh(Provider::Codex, Some("pro_plus"));
        snapshot.providers[Provider::Codex.index()].sources = 3;
        snapshot.providers[Provider::Cursor.index()] = fresh(Provider::Cursor, Some("pro"));
        let mut missing = ProviderStatus::empty(Provider::Kiro);
        missing.detail = "Sign in expired".into();
        snapshot.providers[Provider::Kiro.index()] = missing;
        let mut known = ProviderStatus::empty(Provider::Antigravity);
        known.detail = format!("3 history files{}local", analytics::separator());
        known.detected = true;
        snapshot.providers[Provider::Antigravity.index()] = known;

        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("usage.csv");
        std::fs::write(&csv, CSV).unwrap();
        let config = Config {
            cursor_imports: vec![csv],
            ..Config::default()
        };
        let view = reading(config).open(&snapshot);
        assert_eq!(view.claude_code.caption.as_deref(), Some("1 file | Live"));
        assert_eq!(view.chatgpt.caption.as_deref(), Some("3 files | Pro Plus"));
        assert_eq!(view.cursor.caption.as_deref(), Some("CSV | Pro"));
        assert_eq!(view.kiro.caption.as_deref(), Some("Sign in expired"));
        assert_eq!(view.antigravity.caption.as_deref(), Some("No sign-in"));
        for row in [
            &view.chatgpt,
            &view.claude_code,
            &view.cursor,
            &view.kiro,
            &view.antigravity,
        ] {
            assert_authored(row.caption.as_deref().unwrap());
        }
    }

    #[test]
    fn offline_checked_account_says_cached() {
        let config = Config {
            online: false,
            ..Config::default()
        };
        let mut snapshot = Snapshot::default();
        snapshot.providers[Provider::OpenCode.index()].checked_at = Some(Utc::now());
        let view = reading(config).recall(&snapshot);
        assert_eq!(view.opencode.caption.as_deref(), Some("Cached"));
        assert_eq!(view.chatgpt.caption.as_deref(), Some("Paused"));
    }

    #[test]
    fn privacy_hides_paths_and_keeps_the_hint() {
        let dir = tempfile::tempdir().unwrap();
        let hint = dir.path().join("hint");
        let other = dir.path().join("other");
        std::fs::create_dir_all(&hint).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let mut homes = std::array::from_fn(|_| Vec::new());
        homes[Provider::Kiro.index()] = vec![hint.clone(), other];
        let mut config = Config {
            privacy: true,
            ..Config::default()
        };
        config.custom_homes[Provider::Kiro.index()] = vec![hint.clone()];
        let view = paint(&config, &Snapshot::default(), &homes);
        assert_eq!(view.kiro.hint, Some(Hint::Folder(hint)));
        assert_eq!(view.kiro.hint_label.as_deref(), Some("Hidden"));
        assert_eq!(view.kiro.found, vec!["Hidden".to_string()]);
        assert_authored(view.kiro.hint_label.as_deref().unwrap());
    }

    #[test]
    fn a_hint_is_not_repeated_as_a_found_home() {
        let dir = tempfile::tempdir().unwrap();
        let hint = dir.path().join("hint");
        let other = dir.path().join("other");
        std::fs::create_dir_all(&hint).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let mut homes = std::array::from_fn(|_| Vec::new());
        homes[Provider::Codex.index()] = vec![hint.clone(), other.clone()];
        let mut config = Config::default();
        config.custom_homes[Provider::Codex.index()] = vec![hint.clone()];
        let view = paint(&config, &Snapshot::default(), &homes);
        let label = path_label(&hint, false);
        assert_eq!(view.chatgpt.found, vec![path_label(&other, false)]);
        assert_eq!(view.chatgpt.hint_label.as_deref(), Some(label.as_str()));
        assert!(!forbidden(&view.chatgpt.found[0]));
    }

    #[test]
    fn empty_paths_and_dash_glyphs_stay_plain() {
        assert_eq!(path_label(Path::new(""), false), analytics::empty_value());
        assert_eq!(analytics::empty_value(), "-");
        assert_eq!(path_label(Path::new(""), true), "Hidden");
        let dashed = path_label(Path::new("usage\u{2014}file\u{2192}next"), false);
        assert_eq!(dashed, "usage-file-next");
        assert!(dashed.is_ascii());
        assert!(!forbidden(&dashed));
        let dotted = path_label(Path::new("a\u{00b7}b\u{2026}c"), false);
        assert_eq!(dotted, "a|b...c");
        assert!(dotted.is_ascii());
    }

    #[test]
    fn clearing_a_hint_and_a_missing_folder_round_trip() {
        let missing = PathBuf::from("not-created-yet");
        let mut app = reading(Config::default());
        let mut choices = app.draft();
        choices.opencode.hint = Some(Hint::Folder(missing.clone()));
        choices.chatgpt.hint = None;
        let view = app.choose(&Snapshot::default(), choices).unwrap();
        assert_eq!(view.opencode.hint, Some(Hint::Folder(missing.clone())));
        assert!(view.opencode.hint_label.is_some());
        assert_eq!(
            app.config().custom_homes[Provider::OpenCode.index()],
            vec![missing]
        );
        let mut choices = Choices::from(&view);
        choices.opencode.hint = None;
        let view = app.choose(&Snapshot::default(), choices).unwrap();
        assert!(view.opencode.hint.is_none());
        assert!(view.opencode.hint_label.is_none());
        assert!(app.config().custom_homes[Provider::OpenCode.index()].is_empty());
    }

    #[test]
    fn an_off_row_hides_discovered_homes() {
        let mut config = Config::default();
        for provider in Provider::ALL {
            config.enabled[provider.index()] = false;
        }
        let view = reading(config).open(&Snapshot::default());
        for provider in Provider::ALL {
            let row = view.row(provider);
            assert!(!row.on);
            assert!(row.found.is_empty());
            assert!(row.hint_label.is_none());
            assert!(row.caption.is_none());
        }
    }

    #[test]
    fn a_status_line_with_a_separator_is_not_an_error_caption() {
        let sep = analytics::separator();
        assert!(!reported_error(&format!(
            "Account history{sep}across your machines"
        )));
        assert!(!reported_error(&format!(
            "Detected on this device{sep}no local usage yet"
        )));
        assert!(!reported_error(&format!("3 history files{sep}local")));
        assert!(reported_error("Sign in expired"));
        assert!(!reported_error(""));
        let detail = "Limit reached \u{2014} retry".to_string();
        let plain = plain_text(&detail);
        assert!(!forbidden(&plain));
        assert!(plain.contains('-'));
    }
}
