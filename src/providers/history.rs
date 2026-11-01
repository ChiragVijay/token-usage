use crate::model::{Provider, Tokens, UsageRecord};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

pub fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
        .or_else(|| {
            value.parse::<i64>().ok().and_then(|v| {
                DateTime::from_timestamp_millis(if v < 10_000_000_000 {
                    v.saturating_mul(1000)
                } else {
                    v
                })
            })
        })
}
pub fn uint(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| {
            value.as_f64().and_then(|number| {
                (number.is_finite() && number >= 0. && number <= u64::MAX as f64)
                    .then_some(number as u64)
            })
        })
        .or_else(|| value.as_str().and_then(|v| v.replace(',', "").parse().ok()))
        .unwrap_or(0)
}
fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("")
}
fn project(value: &str) -> String {
    value
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .into()
}
fn codex_tokens(value: &Value) -> Tokens {
    let read = uint(&value["cached_input_tokens"]);
    Tokens {
        input: uint(&value["input_tokens"]).saturating_sub(read),
        output: uint(&value["output_tokens"]),
        cache_read: read,
        cache_write: 0,
    }
}
fn delta(value: &Tokens, old: &Tokens) -> Tokens {
    Tokens {
        input: value.input.saturating_sub(old.input),
        output: value.output.saturating_sub(old.output),
        cache_read: value.cache_read.saturating_sub(old.cache_read),
        cache_write: value.cache_write.saturating_sub(old.cache_write),
    }
}
pub struct Parsed {
    pub records: Vec<UsageRecord>,
    pub skipped: usize,
}

const MAX_HISTORY_LINE_BYTES: u64 = 8 * 1024 * 1024;

pub fn read_jsonl(path: &Path, provider: Provider) -> Result<Parsed> {
    let mut parsed = Parsed {
        records: vec![],
        skipped: 0,
    };
    let mut model = "unknown".to_owned();
    let mut session = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut cwd = String::new();
    let mut previous = Tokens::default();
    let mut previous_total: Option<u64> = None;
    let mut claude: BTreeMap<String, UsageRecord> = BTreeMap::new();
    // Paginated rollouts report each response in token_usage_record. Older
    // rollouts only emit cumulative token_count snapshots. A file can carry
    // both for the same responses, so the two sets are never added together.
    let mut codex_usage: BTreeMap<String, UsageRecord> = BTreeMap::new();
    let mut codex_counts: Vec<UsageRecord> = Vec::new();
    let mut reader = BufReader::new(File::open(path)?);
    loop {
        let mut line = Vec::new();
        if reader
            .by_ref()
            .take(MAX_HISTORY_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)?
            == 0
        {
            break;
        }
        if line.len() as u64 > MAX_HISTORY_LINE_BYTES {
            if line.last() != Some(&b'\n') {
                reader.skip_until(b'\n')?;
            }
            parsed.skipped += 1;
            continue;
        }
        let line = match String::from_utf8(line) {
            Ok(line) => line,
            Err(_) => {
                parsed.skipped += 1;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                parsed.skipped += 1;
                continue;
            }
        };
        if provider == Provider::Codex {
            let payload = &value["payload"];
            if text(&value, "type") == "session_meta" {
                if !text(payload, "id").is_empty() {
                    session = text(payload, "id").into();
                }
                cwd = project(text(payload, "cwd"));
                if !text(payload, "model").is_empty() {
                    model = text(payload, "model").into();
                }
            }
            if text(&value, "type") == "turn_context" && !text(payload, "model").is_empty() {
                model = text(payload, "model").into();
            }
            if text(&value, "type") == "token_usage_record" {
                let Some(timestamp) = date(text(&value, "timestamp")) else {
                    parsed.skipped += 1;
                    continue;
                };
                let usage = &payload["usage"];
                if !usage.is_object() {
                    continue;
                }
                let tokens = codex_tokens(usage);
                if tokens.total() == 0 {
                    continue;
                }
                let response_id = text(payload, "response_id");
                let id = if response_id.is_empty() {
                    hash(&format!("codex-usage:{session}:{timestamp}:{tokens:?}"))
                } else {
                    // Replayed responses keep this id, including across rollout files.
                    hash(&format!("codex-usage:{response_id}"))
                };
                if codex_usage
                    .get(&id)
                    .is_some_and(|old| old.tokens.total() >= tokens.total())
                {
                    continue;
                }
                codex_usage.insert(
                    id.clone(),
                    UsageRecord {
                        id,
                        provider,
                        timestamp,
                        model: model.clone(),
                        session: session.clone(),
                        project: cwd.clone(),
                        tokens,
                        reported_cost: None,
                        source: path.to_string_lossy().into_owned(),
                    },
                );
                continue;
            }
            if text(payload, "type") != "token_count" {
                continue;
            }
            let Some(timestamp) = date(text(&value, "timestamp")) else {
                parsed.skipped += 1;
                continue;
            };
            let info = &payload["info"];
            if info.is_null() {
                continue;
            }
            let total = &info["total_token_usage"];
            let last = &info["last_token_usage"];
            let tokens = if total.is_object() {
                let current = codex_tokens(total);
                let total_count = uint(&total["total_tokens"]).max(current.total());
                if previous_total == Some(total_count) {
                    continue;
                }
                let tokens = if previous_total.is_some_and(|old| total_count < old) {
                    if last.is_object() {
                        codex_tokens(last)
                    } else {
                        current.clone()
                    }
                } else {
                    delta(&current, &previous)
                };
                previous = current;
                previous_total = Some(total_count);
                tokens
            } else if last.is_object() {
                codex_tokens(last)
            } else {
                continue;
            };
            if tokens.total() == 0 {
                continue;
            }
            let id = hash(&format!(
                "codex:{session}:{timestamp}:{previous_total:?}:{tokens:?}"
            ));
            codex_counts.push(UsageRecord {
                id,
                provider,
                timestamp,
                model: model.clone(),
                session: session.clone(),
                project: cwd.clone(),
                tokens,
                reported_cost: None,
                source: path.to_string_lossy().into_owned(),
            });
        } else if provider == Provider::Claude {
            if text(&value, "type") != "assistant" {
                continue;
            }
            let message = &value["message"];
            let usage = &message["usage"];
            if !usage.is_object() {
                continue;
            }
            let Some(timestamp) = date(text(&value, "timestamp")) else {
                parsed.skipped += 1;
                continue;
            };
            let message_id = text(message, "id");
            let session_id = if text(&value, "sessionId").is_empty() {
                session.as_str()
            } else {
                text(&value, "sessionId")
            };
            let request = text(&value, "requestId");
            let identity = if message_id.is_empty() {
                text(&value, "uuid")
            } else {
                message_id
            };
            let id = if identity.is_empty() {
                hash(&format!("claude:{session_id}:{timestamp}:{usage}"))
            } else {
                hash(&format!("claude:{identity}:{request}"))
            };
            let tokens = Tokens {
                input: uint(&usage["input_tokens"]),
                output: uint(&usage["output_tokens"]),
                cache_read: uint(&usage["cache_read_input_tokens"]),
                cache_write: uint(&usage["cache_creation_input_tokens"]),
            };
            let reported_cost = value["costUSD"]
                .as_f64()
                .filter(|cost| cost.is_finite() && *cost >= 0.0);
            if tokens.total() == 0 && reported_cost.is_none() {
                continue;
            }
            // Streaming content blocks repeat the same message. Keep the fullest usage.
            if claude
                .get(&id)
                .is_some_and(|old| old.tokens.total() > tokens.total())
            {
                continue;
            }
            claude.insert(
                id.clone(),
                UsageRecord {
                    id,
                    provider,
                    timestamp,
                    model: if text(message, "model").is_empty() {
                        "unknown"
                    } else {
                        text(message, "model")
                    }
                    .into(),
                    session: session_id.into(),
                    project: project(text(&value, "cwd")),
                    tokens,
                    reported_cost,
                    source: path.to_string_lossy().into_owned(),
                },
            );
        }
    }
    let usage_total: u64 = codex_usage
        .values()
        .map(|record| record.tokens.total())
        .sum();
    let count_total: u64 = codex_counts
        .iter()
        .map(|record| record.tokens.total())
        .sum();
    // Keep the fuller account. Summing them doubles a rollout that writes both.
    if !codex_usage.is_empty() && usage_total >= count_total {
        parsed.records.extend(codex_usage.into_values());
    } else {
        parsed.records.extend(codex_counts);
    }
    parsed.records.extend(claude.into_values());
    Ok(parsed)
}

pub fn read_cursor_csv(path: &Path) -> Result<Vec<UsageRecord>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers: Vec<String> = reader
        .headers()?
        .iter()
        .map(|h| {
            h.trim()
                .trim_start_matches('\u{feff}')
                .to_lowercase()
                .replace([' ', '_', '-'], "")
        })
        .collect();
    let index = |keys: &[&str]| headers.iter().position(|h| keys.contains(&h.as_str()));
    let timestamp =
        index(&["date", "timestamp"]).context("CSV needs a Date or Timestamp column")?;
    let model = index(&["model"]).context("CSV needs a Model column")?;
    let input = index(&[
        "inputtokens",
        "inputtokens(w/ocachewrite)",
        "uncachedinputtokens",
    ]);
    let output = index(&["outputtokens"]);
    let read = index(&["cachereadtokens", "cache(d)readtokens"]);
    let write = index(&[
        "cachewritetokens",
        "cachecreationtokens",
        "inputtokens(w/cachewrite)",
    ]);
    let cost = index(&["cost", "cost($)", "totalcost", "costusd"]);
    if input.is_none() && output.is_none() && cost.is_none() {
        bail!("CSV needs token or cost columns from Cursor's usage export");
    }
    let mut records = vec![];
    let mut occurrences = HashMap::new();
    for row in reader.records() {
        let row = row?;
        let field = |index: Option<usize>| index.and_then(|i| row.get(i)).unwrap_or("").trim();
        let timestamp =
            date(row.get(timestamp).unwrap_or("")).context("CSV contains an invalid timestamp")?;
        let number = |index| -> Result<u64> {
            let value = field(index);
            if value.is_empty() {
                Ok(0)
            } else {
                Ok(value
                    .replace(',', "")
                    .parse::<u64>()
                    .context("CSV token counts must be nonnegative integers")?)
            }
        };
        let tokens = Tokens {
            input: number(input)?,
            output: number(output)?,
            cache_read: number(read)?,
            cache_write: number(write)?,
        };
        let reported_cost =
            if field(cost).is_empty() || field(cost).eq_ignore_ascii_case("included") {
                None
            } else {
                let cost: f64 = field(cost)
                    .trim_start_matches('$')
                    .replace(',', "")
                    .parse()
                    .context("CSV contains an invalid cost")?;
                if !cost.is_finite() || cost < 0.0 {
                    bail!("CSV costs must be nonnegative");
                }
                Some(cost)
            };
        let model = row.get(model).unwrap_or("unknown").to_owned();
        let key = hash(&format!(
            "cursor:{timestamp}:{model}:{tokens:?}:{reported_cost:?}"
        ));
        let occurrence = occurrences.entry(key.clone()).or_insert(0);
        let id = format!("{key}:{occurrence}");
        *occurrence += 1;
        records.push(UsageRecord {
            id,
            provider: Provider::Cursor,
            timestamp,
            model,
            session: String::new(),
            project: "Cursor account".into(),
            tokens,
            reported_cost,
            source: path.to_string_lossy().into_owned(),
        });
    }
    Ok(records)
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn oversized_history_lines_are_skipped_without_losing_later_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut file = File::create(&path).unwrap();
        let block = [b'x'; 8192];
        for _ in 0..(MAX_HISTORY_LINE_BYTES / block.len() as u64 + 1) {
            file.write_all(&block).unwrap();
        }
        writeln!(file).unwrap();
        writeln!(file, "{}", serde_json::json!({
            "timestamp":"2026-10-10T12:00:00Z",
            "type":"event_msg",
            "payload":{"type":"token_count", "info":{"last_token_usage":{"input_tokens":1,"output_tokens":2}}}
        })).unwrap();
        drop(file);
        let parsed = read_jsonl(&path, Provider::Codex).unwrap();
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].tokens.total(), 3);
    }

    fn write_rollout(path: &std::path::Path, lines: &[&str]) {
        let mut file = File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[test]
    fn paginated_rollouts_keep_token_usage_records_when_token_count_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        write_rollout(
            &path,
            &[
                r#"{"timestamp":"2026-10-02T09:00:00.000Z","type":"session_meta","payload":{"id":"thread-1","cwd":"/srv/proj"}}"#,
                r#"{"timestamp":"2026-10-02T09:00:01.200Z","type":"turn_context","payload":{"model":"gpt-5.5-codex"}}"#,
                r#"{"timestamp":"2026-10-02T09:00:08.100Z","type":"token_usage_record","payload":{"response_id":"resp_1","usage":{"input_tokens":900,"cached_input_tokens":0,"output_tokens":40,"reasoning_output_tokens":10,"total_tokens":940},"turn_token_usage":{"input_tokens":9000,"output_tokens":400,"total_tokens":9400}}}"#,
            ],
        );
        let parsed = read_jsonl(&path, Provider::Codex).unwrap();
        assert_eq!(parsed.skipped, 0);
        assert_eq!(parsed.records.len(), 1);
        let record = &parsed.records[0];
        // Reasoning is already inside output. The running turn total is not added.
        assert_eq!(record.tokens.input, 900);
        assert_eq!(record.tokens.output, 40);
        assert_eq!(record.tokens.total(), 940);
        assert_eq!(record.model, "gpt-5.5-codex");
        assert_eq!(record.session, "thread-1");
        assert_eq!(record.project, "proj");
    }

    #[test]
    fn token_usage_records_and_token_count_snapshots_are_not_both_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        write_rollout(
            &path,
            &[
                r#"{"timestamp":"2026-10-02T09:00:08.100Z","type":"token_usage_record","payload":{"response_id":"resp_1","usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":20,"total_tokens":120}}}"#,
                r#"{"timestamp":"2026-10-02T09:00:08.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":20,"total_tokens":120},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":20,"total_tokens":120}}}}"#,
            ],
        );
        let parsed = read_jsonl(&path, Provider::Codex).unwrap();
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].tokens.total(), 120);
        let again = read_jsonl(&path, Provider::Codex).unwrap();
        assert_eq!(parsed.records[0].id, again.records[0].id);
    }

    #[test]
    fn repeated_response_ids_count_once_and_cumulative_snapshots_still_win_when_larger() {
        let dir = tempfile::tempdir().unwrap();
        let replay = dir.path().join("replay.jsonl");
        write_rollout(
            &replay,
            &[
                r#"{"timestamp":"2026-10-02T09:00:08.100Z","type":"token_usage_record","payload":{"response_id":"resp_1","usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}"#,
                r#"{"timestamp":"2026-10-02T09:01:08.100Z","type":"token_usage_record","payload":{"response_id":"resp_1","usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}"#,
            ],
        );
        let parsed = read_jsonl(&replay, Provider::Codex).unwrap();
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].tokens.total(), 120);

        let mixed = dir.path().join("mixed.jsonl");
        write_rollout(
            &mixed,
            &[
                r#"{"timestamp":"2026-10-02T09:00:08.100Z","type":"token_usage_record","payload":{"response_id":"resp_1","usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}"#,
                r#"{"timestamp":"2026-10-02T09:00:08.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}}"#,
                r#"{"timestamp":"2026-10-02T09:05:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":50,"total_tokens":350}}}}"#,
            ],
        );
        let parsed = read_jsonl(&mixed, Provider::Codex).unwrap();
        assert_eq!(
            parsed
                .records
                .iter()
                .map(|record| record.tokens.total())
                .sum::<u64>(),
            350
        );
    }
}
