use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;
use token_usage::{
    config::Config,
    model::{Provider, Snapshot},
    providers::{antigravity, kiro, opencode},
    store::Store,
};

#[test]
fn old_settings_and_cache_preserve_original_accounts() {
    let config: Config = serde_json::from_value(json!({"enabled":[false,true,false],"custom_homes":[["codex-home"],[],["cursor-home"]],"dark":true})).unwrap();
    assert_eq!(config.enabled, [false, true, false, true, true, true]);
    assert_eq!(config.custom_homes[0][0].to_string_lossy(), "codex-home");
    assert!(config.custom_homes[5].is_empty());
    assert_eq!(
        serde_json::from_value::<Config>(serde_json::to_value(&config).unwrap())
            .unwrap()
            .enabled,
        config.enabled
    );
    let dir = TempDir::new().unwrap();
    let store = Store::at(&dir.path().join("cache.db")).unwrap();
    let mut old = Snapshot::default();
    old.providers.truncate(3);
    old.providers.reverse();
    old.providers[2].plan = Some("Existing plan".into());
    store.save(&old).unwrap();
    let migrated = store.load().unwrap().unwrap();
    assert_eq!(migrated.providers.len(), Provider::COUNT);
    assert_eq!(migrated.providers[0].plan.as_deref(), Some("Existing plan"));
    for provider in Provider::ALL {
        assert_eq!(migrated.providers[provider.index()].provider, provider);
    }
}

#[test]
fn kiro_credits_preserve_precision_and_separate_trial_and_bonus() {
    let body = json!({"nextDateReset":1785542400.0,"usageBreakdownList":[
        {"resourceType":"OTHER","currentUsage":1,"usageLimit":10},
        {"resourceType":"CREDIT","currentUsageWithPrecision":9943.38,"usageLimitWithPrecision":10000.,
        "freeTrialUsage":{"currentUsage":10.5,"usageLimit":500.,"freeTrialStatus":"ACTIVE","expiryDate":"2026-11-01T00:00:00Z"},
        "bonuses":[{"currentUsage":1,"usageLimit":25,"status":"EXPIRED"},{"currentUsage":2,"usageLimit":50,"status":"ACTIVE"}]}]});
    let limits = kiro::limits(&body);
    assert_eq!(limits.len(), 3);
    assert!((limits[0].used_percent - 99.4338).abs() < 1e-8);
    assert_eq!(limits[0].credits.as_ref().unwrap().used, 9943.38);
    assert_eq!(limits[0].resets_at.unwrap().timestamp(), 1785542400);
    assert_eq!(limits[1].used_percent, 2.1);
    assert!(kiro::limits(&json!({"usageBreakdownList":[{"resourceType":"CREDIT","currentUsage":3,"usageLimit":0}]})).is_empty());
    assert!(
        kiro::limits(&json!({"usageBreakdownList":[{"resourceType":"CREDIT","usageLimit":100}]}))
            .is_empty()
    );
}

#[test]
fn kiro_ide_cache_keeps_its_original_capture_time() {
    let dir = TempDir::new().unwrap();
    let storage = dir.path().join("User/globalStorage");
    fs::create_dir_all(&storage).unwrap();
    let db = Connection::open(storage.join("state.vscdb")).unwrap();
    db.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value TEXT)")
        .unwrap();
    db.execute("INSERT INTO ItemTable VALUES ('kiro.kiroAgent',?1)",[json!({"kiro.resourceNotifications.usageState":{"timestamp":1775500185544i64,"usageBreakdowns":[{"type":"CREDIT","currentUsage":10,"usageLimit":50}]}}).to_string()]).unwrap();
    let before: String = db
        .query_row("SELECT value FROM ItemTable", [], |row| row.get(0))
        .unwrap();
    let cached = kiro::cached_limits(&[dir.path().into()]).unwrap().unwrap();
    assert_eq!(cached.checked_at.timestamp_millis(), 1775500185544);
    assert_eq!(cached.limits[0].used_percent, 20.);
    assert_eq!(
        before,
        db.query_row::<String, _, _>("SELECT value FROM ItemTable", [], |row| row.get(0))
            .unwrap()
    );
}

fn open_db(dir: &TempDir, channel: &str) -> (std::path::PathBuf, Connection) {
    let path = dir.path().join(format!("opencode{channel}.db"));
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT); CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,type TEXT,data TEXT); CREATE TABLE session(id TEXT,directory TEXT)").unwrap();
    db.execute(
        "INSERT INTO session VALUES('session','project-directory')",
        [],
    )
    .unwrap();
    (path, db)
}
fn message(provider: &str, cost: f64) -> Value {
    json!({"role":"assistant","providerID":provider,"modelID":"model-a","cost":cost,"tokens":{"input":100,"output":30,"reasoning":7,"cache":{"read":20,"write":5}},"time":{"completed":1791028800000i64}})
}
fn credential_table(db: &Connection) {
    db.execute_batch("CREATE TABLE credential(id TEXT,integration_id TEXT,active INTEGER,time_created INTEGER,time_updated INTEGER,value TEXT)").unwrap();
}

#[test]
fn opencode_unions_versions_and_counts_each_hosted_message_once() {
    let dir = TempDir::new().unwrap();
    let (path, db) = open_db(&dir, "");
    db.execute(
        "INSERT INTO message VALUES('same','session',1791028800000,?1)",
        [message("opencode", 0.).to_string()],
    )
    .unwrap();
    let v2 = json!({"model":{"id":"model-a","providerID":"opencode"},"tokens":{"input":100,"output":30,"reasoning":7,"cache":{"read":20,"write":5}},"cost":0.});
    db.execute(
        "INSERT INTO session_message VALUES('same','session',1791028800000,'assistant',?1)",
        [v2.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO message VALUES('byok','session',1791028800000,?1)",
        [message("openai", 3.).to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO session_message VALUES('compact','session',1791028800000,'compaction',?1)",[json!({"model":{"id":"model-b","providerID":"opencode-go"},"status":"completed","tokens":{"input":12,"output":4},"cost":0.25}).to_string()]).unwrap();
    db.execute("INSERT INTO session_message VALUES('incomplete','session',1791028800000,'compaction',?1)",[json!({"model":{"providerID":"opencode-go"},"status":"pending","tokens":{"input":1000},"cost":10.}).to_string()]).unwrap();
    let rows = opencode::history(&path).unwrap();
    assert_eq!(rows.len(), 2);
    let row = rows.iter().find(|row| row.model == "model-a").unwrap();
    assert_eq!(row.tokens.total(), 162);
    assert_eq!(row.tokens.output, 37);
    assert_eq!(row.reported_cost, Some(0.));
    assert_eq!(row.project, "project-directory");
    assert!(rows.iter().all(|row| row.provider == Provider::OpenCode));
    assert_eq!(opencode::history(&path).unwrap().len(), 2);
}

#[test]
fn opencode_credential_table_respects_logout_and_active_account_order() {
    let dir = TempDir::new().unwrap();
    let (_, db) = open_db(&dir, "");
    fs::write(
        dir.path().join("auth.json"),
        json!({"opencode-go":{"type":"api","key":"obsolete-key"}}).to_string(),
    )
    .unwrap();
    assert_eq!(
        opencode::go_key(&[dir.path().into()]).unwrap().as_deref(),
        Some("obsolete-key")
    );
    credential_table(&db);
    assert_eq!(opencode::go_key(&[dir.path().into()]).unwrap(), None);
    db.execute(
        "INSERT INTO credential VALUES('inactive','opencode-go',0,1,99,?1)",
        [json!({"type":"api","key":"inactive-key"}).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO credential VALUES('active','opencode-go',1,1,2,?1)",
        [json!({"type":"api","key":"current-key"}).to_string()],
    )
    .unwrap();
    assert_eq!(
        opencode::go_key(&[dir.path().into()]).unwrap().as_deref(),
        Some("current-key")
    );
    db.execute("DELETE FROM credential WHERE active=1", [])
        .unwrap();
    assert_eq!(opencode::go_key(&[dir.path().into()]).unwrap(), None);
}

#[test]
fn opencode_codex_attribution_requires_channel_oauth_and_completion() {
    let dir = TempDir::new().unwrap();
    let (path, db) = open_db(&dir, "");
    credential_table(&db);
    db.execute(
        "INSERT INTO credential VALUES('login','openai',1,1791028800000,1791028800000,?1)",
        [json!({"type":"oauth","access":"test-access","refresh":"test-refresh"}).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO message VALUES('codex','session',1791028800000,?1)",
        [message("openai", 0.).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO message VALUES('api','session',1791028800000,?1)",
        [message("openai", 0.5).to_string()],
    )
    .unwrap();
    let mut incomplete = message("openai", 0.);
    incomplete.as_object_mut().unwrap().remove("time");
    db.execute(
        "INSERT INTO message VALUES('unfinished','session',1791028800000,?1)",
        [incomplete.to_string()],
    )
    .unwrap();
    let rows = opencode::codex_history(&path).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].provider, Provider::Codex);
    assert_eq!(rows[0].reported_cost, None);
    let (next, nextdb) = open_db(&dir, "-next");
    credential_table(&nextdb);
    nextdb
        .execute(
            "INSERT INTO credential VALUES('key','openai',1,1,1,?1)",
            [json!({"type":"api","key":"api-key"}).to_string()],
        )
        .unwrap();
    nextdb
        .execute(
            "INSERT INTO message VALUES('api-zero','session',1791028800000,?1)",
            [message("openai", 0.).to_string()],
        )
        .unwrap();
    assert!(opencode::codex_history(&next).unwrap().is_empty());
    db.execute("DELETE FROM credential", []).unwrap();
    fs::write(
        dir.path().join("auth.json"),
        json!({"openai":{"type":"oauth","access":"obsolete"}}).to_string(),
    )
    .unwrap();
    assert!(opencode::codex_history(&path).unwrap().is_empty());
}

#[test]
fn opencode_usage_percent_is_used_and_placeholder_reset_is_removed_only_for_session() {
    let now = DateTime::parse_from_rfc3339("2026-10-03T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let body = json!({"usage":{"rolling":{"percent":0,"resetsAt":"2026-10-03T17:00:00Z"},"weekly":{"percent":30,"resetsAt":"2026-10-05T00:00:00Z"},"monthly":{"percent":60,"resetsAt":"2026-11-01T00:00:00Z"}}});
    let limits = opencode::limits(&body, now).unwrap();
    assert_eq!(limits[0].resets_at, None);
    assert_eq!(limits[1].remaining(), 70.);
    assert!(limits[2].resets_at.is_some());
    let mut used = body;
    used["usage"]["rolling"]["resetsAt"] = json!("2026-10-03T16:59:00Z");
    assert!(opencode::limits(&used, now).unwrap()[0].resets_at.is_some());
    assert!(opencode::limits(&json!({"usage":{}}), now).is_err());
}

#[test]
fn antigravity_shared_summary_buckets_are_exact_and_missing_stays_unknown() {
    let body = json!({"response":{"groups":[{"buckets":[{"bucketId":"gemini-5h","remainingFraction":0.75},{"bucketId":"gemini-weekly","remainingFraction":0.5},{"bucketId":"3p-5h","remainingFraction":0.2},{"bucketId":"3p-weekly"},{"bucketId":"gemini-image-5h","remainingFraction":0.1},{"bucketId":"gemini-5h","remainingFraction":0.01}]}]}});
    let limits = antigravity::summary(&body).unwrap();
    assert_eq!(limits.len(), 3);
    assert_eq!(limits[0].remaining(), 75.);
    assert_eq!(limits[1].remaining(), 50.);
    assert_eq!(limits[2].remaining(), 20.);
    assert!(
        antigravity::summary(&json!({"groups":[]}))
            .unwrap()
            .is_empty()
    );
    assert!(antigravity::summary(&json!({"other":[]})).is_none());
}

#[test]
fn antigravity_legacy_quotas_merge_each_pool_by_worst_remaining() {
    let limits = antigravity::legacy(
        &json!({"models":{"gemini-pro":{"quotaInfo":{"remainingFraction":0.9}},"gemini-flash":{"quotaInfo":{"remainingFraction":0.4}},"claude-sonnet":{"quotaInfo":{"remainingFraction":0.2}},"gpt-oss":{"quotaInfo":{"remainingFraction":0.6}},"claude-missing":{},"gemini-internal":{"isInternal":true,"quotaInfo":{"remainingFraction":0.01}}}}),
    );
    assert_eq!(limits.len(), 2);
    assert_eq!(limits[0].remaining(), 40.);
    assert_eq!(limits[1].remaining(), 20.);
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = vec![];
    while value >= 128 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}
fn integer(field: u64, value: u64) -> Vec<u8> {
    [varint(field << 3), varint(value)].concat()
}
fn bytes(field: u64, data: &[u8]) -> Vec<u8> {
    [
        varint((field << 3) | 2),
        varint(data.len() as u64),
        data.to_vec(),
    ]
    .concat()
}
fn generation_blob(with_time: bool) -> Vec<u8> {
    generation_blob_at(with_time.then_some(1791028800))
}
fn generation_blob_at(stamp: Option<u64>) -> Vec<u8> {
    let usage = [
        integer(1, 10),
        integer(2, 100),
        integer(3, 30),
        integer(5, 40),
    ]
    .concat();
    let mut event = [
        bytes(19, b"gemini-default"),
        bytes(21, b"Gemini 3.1 Pro"),
        bytes(4, &usage),
    ]
    .concat();
    if let Some(stamp) = stamp {
        event.extend(bytes(9, &bytes(4, &integer(1, stamp))));
    }
    bytes(1, &event)
}

#[test]
fn antigravity_generation_counts_system_input_once_and_requires_real_timestamp() {
    let blob = generation_blob(true);
    let (stamp, model, tokens) = antigravity::generation(&blob, None).unwrap();
    assert_eq!(stamp.timestamp(), 1791028800);
    assert_eq!(model, "gemini-3.1-pro");
    assert_eq!(tokens.input, 110);
    assert_eq!(tokens.cache_read, 40);
    assert_eq!(tokens.total(), 180);
    let blob = generation_blob(false);
    assert!(antigravity::generation(&blob, None).is_none());
    assert!(antigravity::generation(&blob, Some(&bytes(1, &integer(1, 1791028800)))).is_some());
    let bookkeeping = bytes(1, &bytes(4, &integer(1, 1000)));
    assert!(
        antigravity::generation(&bookkeeping, Some(&bytes(1, &integer(1, 1791028800)))).is_none()
    );
    for malformed in [
        vec![255; 20],
        vec![10, 255],
        vec![10, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 1],
    ] {
        assert!(antigravity::generation(&malformed, None).is_none());
    }
}

#[test]
fn antigravity_invalid_embedded_timing_uses_only_valid_step_timing() {
    let valid_step = bytes(1, &integer(1, 1791028800));
    for stamp in [0, u64::MAX] {
        let blob = generation_blob_at(Some(stamp));
        assert!(antigravity::generation(&blob, None).is_none());
        assert_eq!(
            antigravity::generation(&blob, Some(&valid_step))
                .unwrap()
                .0
                .timestamp(),
            1791028800
        );
    }
    assert!(
        antigravity::generation(&generation_blob(false), Some(&bytes(1, &integer(1, 0)))).is_none()
    );
}

#[test]
fn antigravity_sqlite_scan_uses_step_timing_and_skips_oversized_blobs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("conversation.db");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE gen_metadata(idx INTEGER,data BLOB);CREATE TABLE steps(idx INTEGER,metadata BLOB)").unwrap();
    db.execute(
        "INSERT INTO gen_metadata VALUES(1,?1)",
        [generation_blob(false)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO steps VALUES(1,?1)",
        [bytes(1, &integer(1, 1791028800))],
    )
    .unwrap();
    db.execute(
        "INSERT INTO gen_metadata VALUES(2,?1)",
        [vec![0u8; 1048577]],
    )
    .unwrap();
    let (records, oversized) = antigravity::history_with_warnings(&path).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(oversized, 1);
    assert_eq!(records[0].reported_cost, None);
    assert_eq!(records[0].provider, Provider::Antigravity);
    assert_eq!(records[0].session, "conversation");
    assert_eq!(records[0].id, antigravity::history(&path).unwrap()[0].id);
}

#[test]
fn missing_sources_are_not_created_and_channel_discovery_excludes_wal() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.db");
    assert!(opencode::history(&missing).is_err());
    assert!(!missing.exists());
    fs::write(dir.path().join("opencode-next.db"), b"").unwrap();
    fs::write(dir.path().join("opencode-next.db-wal"), b"").unwrap();
    assert_eq!(opencode::databases(&[dir.path().into()]).unwrap().len(), 1);
}
