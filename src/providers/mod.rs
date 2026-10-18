pub mod antigravity;
mod auth;
mod claude;
mod codex;
mod cursor;
pub mod discovery;
pub mod history;
mod http;
pub mod kiro;
pub mod live;
mod local;
pub mod opencode;
mod process;
mod proto;
mod scan;

use crate::{
    config::Config,
    model::{Provider, Snapshot},
    store::Store,
};
use anyhow::Result;
use chrono::Utc;
use scan::{LocalScan, scan_local};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

pub fn homes(provider: Provider, config: &Config) -> Vec<PathBuf> {
    if let Some(root) = std::env::var_os(format!(
        "TOKEN_USAGE_{}_HOME",
        provider.key().to_ascii_uppercase()
    )) {
        let mut paths = vec![PathBuf::from(root)];
        paths.extend(config.custom_homes[provider.index()].iter().cloned());
        return distinct_paths(paths);
    }
    let home = dirs::home_dir().unwrap_or_default();
    let default = match provider {
        Provider::Codex => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex")),
        Provider::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude")),
        Provider::Cursor => {
            if cfg!(target_os = "windows") {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData/Roaming"))
                    .join("Cursor")
            } else {
                home.join(".cursor")
            }
        }
        Provider::Kiro => editor_home("Kiro", &home),
        Provider::Antigravity => editor_home("Antigravity", &home),
        Provider::OpenCode => std::env::var_os("OPENCODE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local/share"))
                    .join("opencode")
            }),
    };
    let mut paths = vec![default];
    if provider == Provider::Kiro {
        paths.extend([
            home.join(".kiro"),
            home.join(".aws/sso/cache"),
            // Native Windows CLI uses Local rather than roaming app data.
            kiro::cli_home(&dirs::data_local_dir().unwrap_or_default()),
            dirs::data_dir().unwrap_or_default().join("kiro-cli"),
            home.join(".local/share/kiro-cli"),
            dirs::data_dir().unwrap_or_default().join("amazon-q"),
        ]);
    }
    if provider == Provider::Antigravity {
        paths.push(editor_home("Antigravity IDE", &home));
        paths.push(home.join(".gemini/antigravity-cli"));
        if let Ok(stores) = std::fs::read_dir(home.join(".gemini")) {
            paths.extend(
                stores
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("antigravity")
                    })
                    .map(|entry| entry.path()),
            );
        }
    }
    if provider == Provider::Cursor {
        paths.push(home.join(".cursor"));
        if cfg!(target_os = "macos") {
            paths.push(home.join("Library/Application Support/Cursor"));
        }
    }
    paths.extend(config.custom_homes[provider.index()].iter().cloned());
    distinct_paths(paths)
}
fn distinct_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    paths
}
fn editor_home(name: &str, home: &std::path::Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        dirs::config_dir()
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join(name)
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support").join(name)
    } else {
        dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join(name)
    }
}

pub fn collect(config: &Config, previous: &Snapshot) -> Result<Snapshot> {
    collect_with(config, previous, |_| {})
}

/// Scans every account at once. Allowance checks are published as soon as they
/// return, without waiting for the other accounts or for local history.
pub fn collect_with(
    config: &Config,
    previous: &Snapshot,
    progress: impl Fn(&Snapshot),
) -> Result<Snapshot> {
    crate::pricing::start(config.online);
    let store = Arc::new(Mutex::new(Store::open()?));
    let environment = discovery::Environment::current();
    let mut snapshot = previous.clone();
    snapshot.normalize_providers();
    snapshot.notices.clear();
    for provider in Provider::ALL {
        let status = &mut snapshot.providers[provider.index()];
        if !config.enabled[provider.index()] {
            status.detail = "Paused in Connections".into();
            snapshot
                .records
                .retain(|record| record.provider != provider);
        }
    }

    enum Event {
        Accounts(Provider, Option<String>, Result<live::LiveData, String>),
        History(Provider, LocalScan),
    }
    let (sender, receiver) = mpsc::channel();
    let mut pending = 0;
    for provider in Provider::ALL {
        if !config.enabled[provider.index()] {
            continue;
        }
        let directories = homes(provider, config);
        let mut account_homes = directories.clone();
        if provider == Provider::Codex {
            account_homes.extend(homes(Provider::OpenCode, config));
        }
        let check_account = config.online
            && (snapshot.providers[provider.index()].detected
                || snapshot.providers[provider.index()].connected
                || snapshot.providers[provider.index()].checked_at.is_some()
                || (provider == Provider::Codex
                    && account_homes.iter().any(|home| {
                        home.join("opencode.db").is_file()
                            || home.join("opencode-next.db").is_file()
                    }))
                || !discovery::detect(provider, &directories, &environment).is_empty());
        if check_account {
            pending += 1;
            let sender = sender.clone();
            let directories = account_homes;
            let history = provider == Provider::Cursor && config.cursor_imports.is_empty();
            thread::spawn(move || {
                let fingerprint = live::fingerprint(provider, &directories).ok().flatten();
                let result =
                    live::fetch(provider, &directories, history).map_err(|e| e.to_string());
                let _ = sender.send(Event::Accounts(provider, fingerprint, result));
            });
        }
        pending += 1;
        let sender = sender.clone();
        let config = config.clone();
        let previous_records = snapshot
            .records
            .iter()
            .filter(|record| record.provider == provider)
            .cloned()
            .collect::<Vec<_>>();
        let store = store.clone();
        thread::spawn(move || {
            let environment = discovery::Environment::current();
            let scan = scan_local(
                provider,
                &config,
                &directories,
                &environment,
                &store,
                &previous_records,
            );
            let _ = sender.send(Event::History(provider, scan));
        });
    }
    drop(sender);

    let mut live_error: [Option<String>; Provider::COUNT] = Default::default();
    let mut history_done = [false; Provider::COUNT];
    while pending > 0 {
        let Ok(event) = receiver.recv() else {
            break;
        };
        pending -= 1;
        match event {
            Event::Accounts(provider, fingerprint, result) => {
                let status = &mut snapshot.providers[provider.index()];
                if matches!(
                    provider,
                    Provider::Kiro | Provider::Antigravity | Provider::OpenCode
                ) && (fingerprint.is_none() || status.account_fingerprint != fingerprint)
                {
                    status.limits.clear();
                    status.plan = None;
                    status.checked_at = None;
                }
                status.account_fingerprint = fingerprint;
                match result {
                    Ok(data) => {
                        status.limits = data.limits;
                        status.plan = data.plan;
                        status.checked_at = Some(Utc::now());
                        status.stale = false;
                        status.connected = true;
                        live_error[provider.index()] = None;
                        if provider == Provider::Cursor && config.cursor_imports.is_empty() {
                            snapshot
                                .records
                                .retain(|record| record.provider != provider);
                            snapshot.records.extend(data.records);
                        }
                    }
                    Err(error) => {
                        live_error[provider.index()] = Some(error);
                        status.stale = true;
                    }
                }
            }
            Event::History(provider, scan) => {
                let status = &mut snapshot.providers[provider.index()];
                status.installations = scan.installations;
                status.detected = scan.detected;
                status.sources = scan.sources;
                if scan.sources > 0 {
                    status.connected = true;
                }
                if let Some(cached) = scan.cached_limits
                    && (status.stale || status.checked_at.is_none())
                    && (!config.online || live_error[provider.index()].is_none())
                    && status
                        .checked_at
                        .is_none_or(|checked| cached.checked_at > checked)
                {
                    status.limits = cached.limits;
                    status.checked_at = Some(cached.checked_at);
                    status.stale = true;
                    status.connected = true;
                }
                // Cursor account history arrives with the allowance check. A local
                // scan must not wipe it when no CSV import is selected.
                let cursor_api = provider == Provider::Cursor
                    && config.cursor_imports.is_empty()
                    && config.online;
                if !cursor_api {
                    snapshot
                        .records
                        .retain(|record| record.provider != provider);
                    snapshot.records.extend(scan.records.into_values());
                }
                snapshot.notices.extend(scan.notices);
                history_done[provider.index()] = true;
            }
        }
        for provider in Provider::ALL {
            if config.enabled[provider.index()] {
                describe(
                    &mut snapshot.providers[provider.index()],
                    live_error[provider.index()].as_deref(),
                    history_done[provider.index()],
                    config.online,
                );
            }
        }
        snapshot
            .records
            .sort_by_key(|record| std::cmp::Reverse(record.timestamp));
        snapshot.refreshed_at = Some(Utc::now());
        progress(&snapshot);
    }
    store
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .save(&snapshot)?;
    Ok(snapshot)
}

fn describe(
    status: &mut crate::model::ProviderStatus,
    error: Option<&str>,
    history: bool,
    online: bool,
) {
    if let Some(error) = error
        && (status.checked_at.is_none() || status.stale)
    {
        status.detail = error.to_owned();
        return;
    }
    if !status.stale && status.checked_at.is_some() {
        let sep = crate::analytics::separator();
        status.detail = if status.provider == Provider::Cursor {
            format!("Account history{sep}across your machines")
        } else if status.provider == Provider::Kiro {
            "Live credit allowance".into()
        } else if status.limits.is_empty() {
            format!("Local history{sep}no subscription limits reported")
        } else if status.sources > 0 || !history {
            "Local history + live account limits".into()
        } else {
            "Live account limits".into()
        };
        return;
    }
    if !history {
        return;
    }
    status.detail = if status.sources > 0 {
        if status.provider == Provider::Cursor && !online {
            "Cached Cursor account history".into()
        } else {
            format!(
                "{} history files{}local",
                status.sources,
                crate::analytics::separator()
            )
        }
    } else if status.detected {
        format!(
            "Detected on this device{}no local usage yet",
            crate::analytics::separator()
        )
    } else if status.provider == Provider::Cursor {
        "Import a usage CSV or enable account checks".into()
    } else {
        format!(
            "Use {} to create local history, or add its data folder",
            status.provider.name()
        )
    };
}
