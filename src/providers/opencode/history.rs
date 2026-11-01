use super::auth::openai_credential;
use crate::model::{Provider, Tokens, UsageRecord};
use crate::providers::{
    history::{hash, uint},
    http::token,
    local,
};
use anyhow::Result;
use chrono::DateTime;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub fn history(path: &Path) -> Result<Vec<UsageRecord>> {
    history_for(path, false)
}
pub fn codex_history(path: &Path) -> Result<Vec<UsageRecord>> {
    history_for(path, true)
}
fn history_for(path: &Path, codex: bool) -> Result<Vec<UsageRecord>> {
    let connection = local::database(path)?;
    let oauth = if codex {
        openai_credential(&connection, path)?
    } else {
        None
    };
    if codex
        && oauth.as_ref().is_none_or(|(auth, _)| {
            auth["type"] != "oauth"
                || (token(auth, "/access").is_none() && token(auth, "/refresh").is_none())
        })
    {
        return Ok(vec![]);
    }
    let mut records = BTreeMap::new();
    let mut row_errors = false;
    let mut projects = BTreeMap::<String, String>::new();
    if local::table(&connection, "session")? {
        let has_directory: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session') WHERE name='directory')",
            [],
            |row| row.get(0),
        )?;
        if has_directory {
            let mut query = connection.prepare("SELECT id,directory FROM session")?;
            for row in query.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })? {
                let (id, directory) = row?;
                projects.insert(id, directory);
            }
        }
    }
    // Read v2 first: migrated rows retain their IDs, so v1 duplicates lose.
    for (table, role) in [
        (
            "session_message",
            "(type='assistant' OR (type='compaction' AND json_extract(data,'$.status')='completed'))",
        ),
        ("message", "json_extract(data,'$.role')='assistant'"),
    ] {
        if !local::table(&connection, table)? {
            continue;
        }
        let has_session: bool = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name='session_id')"
            ),
            [],
            |row| row.get(0),
        )?;
        let session = if has_session {
            "session_id"
        } else {
            "json_extract(data,'$.sessionID')"
        };
        // Select accounting fields only; prompt and response content stay in the agent's DB.
        let filter = if codex {
            "AND COALESCE(json_extract(data,'$.model.providerID'),json_extract(data,'$.providerID'))='openai' AND json_extract(data,'$.cost')=0 AND (json_type(data,'$.time.completed') IN ('integer','real') OR json_type(data,'$.finish')='text' OR json_extract(data,'$.status')='completed')"
        } else {
            "AND COALESCE(json_extract(data,'$.model.providerID'),json_extract(data,'$.providerID')) IN ('opencode','opencode-go') AND json_type(data,'$.cost') IN ('integer','real')"
        };
        let sql = format!(
            "SELECT id,CAST(COALESCE(json_extract(data,'$.time.completed'),time_created) AS INTEGER),json_object('session', {session}, 'model',COALESCE(json_extract(data,'$.model.id'),json_extract(data,'$.modelID')),'provider',COALESCE(json_extract(data,'$.model.providerID'),json_extract(data,'$.providerID')),'tokens',json_extract(data,'$.tokens'),'cost',json_extract(data,'$.cost')) FROM {table} WHERE json_valid(data) AND {role} {filter}"
        );
        let mut query = connection.prepare(&sql)?;
        for row in query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            // One unreadable row must not discard the rest of the database.
            let Ok((id, ms, raw)) = row else {
                row_errors = true;
                continue;
            };
            if codex
                && table == "session_message"
                && oauth
                    .as_ref()
                    .and_then(|(_, since)| *since)
                    .is_some_and(|since| ms < since)
            {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&raw) else {
                row_errors = true;
                continue;
            };
            let Some(timestamp) = DateTime::from_timestamp_millis(ms) else {
                row_errors = true;
                continue;
            };
            let session = data["session"].as_str().unwrap_or("").to_owned();
            let usage = &data["tokens"];
            let tokens = Tokens {
                input: uint(&usage["input"]),
                output: uint(&usage["output"]).saturating_add(uint(&usage["reasoning"])),
                cache_read: uint(&usage["cache"]["read"]),
                cache_write: uint(&usage["cache"]["write"]),
            };
            let cost = if codex {
                None
            } else {
                data["cost"]
                    .as_f64()
                    .filter(|cost| cost.is_finite() && *cost >= 0.)
            };
            if tokens.total() == 0 && cost.is_none_or(|cost| cost == 0.) {
                continue;
            }
            let id = format!("opencode:{}", hash(&id));
            records.entry(id.clone()).or_insert(UsageRecord {
                id,
                provider: if codex {
                    Provider::Codex
                } else {
                    Provider::OpenCode
                },
                timestamp,
                model: data["model"].as_str().unwrap_or("Unknown model").into(),
                project: projects.get(&session).cloned().unwrap_or_default(),
                session,
                tokens,
                reported_cost: cost,
                source: path.to_string_lossy().into_owned(),
            });
        }
    }
    if records.is_empty() && row_errors {
        anyhow::bail!("OpenCode history rows could not be read");
    }
    Ok(records.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn database(path: &std::path::Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session_message (
                    id TEXT PRIMARY KEY,
                    session_id TEXT,
                    type TEXT,
                    time_created INTEGER,
                    time_updated INTEGER,
                    data TEXT NOT NULL
                );",
            )
            .unwrap();
    }

    fn insert(path: &std::path::Path, id: &str, created: Option<i64>, data: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                "INSERT INTO session_message (id, session_id, type, time_created, time_updated, data) VALUES (?1, 'ses_1', 'assistant', ?2, ?2, ?3)",
                (id, created, data),
            )
            .unwrap();
    }

    #[test]
    fn one_unreadable_row_does_not_drop_the_rest_of_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        database(&path);
        insert(
            &path,
            "msg_good",
            Some(1_700_000_000_000),
            r#"{"model":{"id":"gpt-5","providerID":"opencode"},"cost":0.25,"tokens":{"input":10.0,"output":4,"reasoning":1,"cache":{"read":2.0,"write":3}}}"#,
        );
        insert(
            &path,
            "msg_bad",
            None,
            r#"{"model":{"id":"gpt-5","providerID":"opencode"},"cost":1,"tokens":{"input":50,"output":5,"reasoning":0,"cache":{"read":0,"write":0}}}"#,
        );
        let records = history(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tokens.input, 10);
        assert_eq!(records[0].tokens.output, 5);
        assert_eq!(records[0].tokens.cache_read, 2);
        assert_eq!(records[0].tokens.cache_write, 3);
        assert_eq!(records[0].tokens.total(), 20);
        assert!((records[0].reported_cost.unwrap() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_database_whose_rows_cannot_be_read_stays_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        database(&path);
        insert(
            &path,
            "msg_bad",
            None,
            r#"{"model":{"id":"gpt-5","providerID":"opencode"},"cost":1,"tokens":{"input":50,"output":5,"reasoning":0,"cache":{"read":0,"write":0}}}"#,
        );
        assert!(history(&path).is_err());
    }
}
