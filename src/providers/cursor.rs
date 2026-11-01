use super::{
    history::{date, hash, uint},
    http::{keychain, read, response, token, window},
    live::LiveData,
};
use crate::model::{LimitWindow, Provider, Tokens, UsageRecord};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

mod auth;

pub fn cursor_limits(body: &Value) -> Vec<LimitWindow> {
    let duration = period_minutes(body);
    let weekly = duration.is_some_and(|minutes| (6 * 1440..=8 * 1440).contains(&minutes));
    [
        (
            "totalPercentUsed",
            if weekly { "Weekly" } else { "Monthly" },
        ),
        ("autoPercentUsed", "Cursor models"),
        ("apiPercentUsed", "Other models"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        if weekly && key != "totalPercentUsed" {
            return None;
        }
        let usage = &body["planUsage"];
        let percent = number(&usage[key]).or_else(|| {
            let (used, limit) = match key {
                "totalPercentUsed" => (
                    number(&usage["totalSpend"]).or_else(|| number(&usage["used"])),
                    number(&usage["limit"]),
                ),
                "autoPercentUsed" => (number(&usage["autoSpend"]), number(&usage["autoLimit"])),
                _ => (number(&usage["apiSpend"]), number(&usage["apiLimit"])),
            };
            Some(used? / limit.filter(|v| *v > 0.)? * 100.)
        })?;
        window(label, &json!(percent), &body["billingCycleEnd"], duration)
    })
    .collect()
}

fn period_minutes(body: &Value) -> Option<u64> {
    let start = date(
        &body["billingCycleStart"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| body["billingCycleStart"].to_string()),
    )?;
    let end = date(
        &body["billingCycleEnd"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| body["billingCycleEnd"].to_string()),
    )?;
    u64::try_from((end - start).num_minutes())
        .ok()
        .filter(|v| *v > 0)
}

/// The billing period is independent of the subscription's payment cadence.
/// A legacy account can also have a separate weekly Grok Bot allowance.
fn account_limits(
    current: &Value,
    plan: &Value,
    sand: &Value,
    summary: &Value,
) -> Result<Vec<LimitWindow>> {
    let info = &plan["planInfo"];
    let weekly = sand["isUnified"].as_bool() == Some(true)
        || info["includedUsagePeriod"] == "INCLUDED_USAGE_PERIOD_WEEKLY"
        || info["includedUsagePeriod"] == 2
        || period_minutes(current).is_some_and(|m| (6 * 1440..=8 * 1440).contains(&m));
    if weekly {
        let shared = window(
            "Weekly",
            &sand["usagePercent"],
            &sand["nextResetTimestampUtc"],
            Some(10080),
        )
        .filter(|_| sand["isUnified"].as_bool() == Some(true));
        let current_is_weekly =
            period_minutes(current).is_some_and(|m| (6 * 1440..=8 * 1440).contains(&m));
        // Never project a stale monthly percentage onto a weekly reset.
        let from_current = if current_is_weekly
            || (period_minutes(current).is_none()
                && (info["includedUsagePeriod"] == "INCLUDED_USAGE_PERIOD_WEEKLY"
                    || info["includedUsagePeriod"] == 2))
        {
            let reset = if info["includedUsageResetsAt"].is_null() && current_is_weekly {
                &current["billingCycleEnd"]
            } else {
                &info["includedUsageResetsAt"]
            };
            cursor_limits(current).into_iter().next().map(|mut w| {
                w.label = "Weekly".into();
                w.duration_minutes = Some(10080);
                w.resets_at = window("Weekly", &json!(w.used_percent), reset, Some(10080))
                    .and_then(|w| w.resets_at);
                w
            })
        } else {
            None
        };
        return shared
            .or(from_current)
            .map(|w| vec![w])
            .context("Cursor's weekly allowance is unavailable. Previous results are retained.");
    }
    let mut limits = cursor_limits(current);
    if limits.is_empty() {
        let fallback = json!({
            "billingCycleStart": summary["billingCycleStart"],
            "billingCycleEnd": summary["billingCycleEnd"],
            "planUsage": summary["individualUsage"]["plan"],
        });
        limits = cursor_limits(&fallback);
        if limits.is_empty() {
            for (path, label) in [
                ("/individualUsage/overall", "Monthly"),
                ("/teamUsage/pooled", "Team monthly"),
            ] {
                if let Some(bucket) = summary.pointer(path)
                    && let Some(limit) = number(&bucket["limit"]).filter(|v| *v > 0.)
                    && let Some(used) = number(&bucket["used"])
                    && let Some(w) = window(
                        label,
                        &json!(used / limit * 100.),
                        &summary["billingCycleEnd"],
                        period_minutes(summary),
                    )
                {
                    limits.push(w);
                }
            }
        }
    }
    if sand["isUnified"].as_bool() != Some(true)
        && let Some(w) = window(
            "Grok Bot weekly",
            &sand["usagePercent"],
            &sand["nextResetTimestampUtc"],
            Some(10080),
        )
    {
        limits.push(w);
    }
    if limits.is_empty() {
        bail!("Cursor returned an unsupported allowance format. Previous results are retained.");
    }
    Ok(limits)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite() && *v >= 0.)
}

pub fn fetch(client: &Client, homes: &[PathBuf], history: bool) -> Result<LiveData> {
    let provider = Provider::Cursor;

    let access = std::env::var("CURSOR_AUTH_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("CURSOR_ACCESS_TOKEN")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            homes.iter().find_map(|home| {
                read(&home.join("auth.json")).and_then(|auth| token(&auth, "/accessToken"))
            })
        });
    let access = match access {
        Some(access) => Some(access),
        None => cursor_desktop_token(homes)?,
    }
    .context("Sign in to Cursor CLI, or import a usage CSV")?;
    let rpc = |method: &str| {
        response(
            client
                .post(format!(
                    "https://api2.cursor.sh/aiserver.v1.DashboardService/{method}"
                ))
                .bearer_auth(&access)
                .header("connect-protocol-version", "1")
                .header("x-cursor-client-type", "cli")
                .json(&json!({})),
            provider,
        )
    };
    let (current, plan, sand) = std::thread::scope(|scope| {
        let current = scope.spawn(|| rpc("GetCurrentPeriodUsage"));
        let plan = scope.spawn(|| rpc("GetPlanInfo"));
        let sand = scope.spawn(|| rpc("GetSandUsageStatus"));
        (
            current.join().unwrap(),
            plan.join().unwrap(),
            sand.join().unwrap(),
        )
    });
    let body = current.as_ref().ok().cloned().unwrap_or(Value::Null);
    let plan = plan.unwrap_or(Value::Null);
    let sand = sand.unwrap_or(Value::Null);
    let summary = if cursor_limits(&body).is_empty() {
        session_cookie(&access)
            .ok()
            .and_then(|(cookie, _)| {
                response(
                    client
                        .get("https://cursor.com/api/usage-summary")
                        .header("Cookie", cookie),
                    provider,
                )
                .ok()
            })
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let limits = account_limits(&body, &plan, &sand, &summary)
        .map_err(|error| current.err().unwrap_or(error))?;
    let records = if history {
        cursor_history(client, &access)?
    } else {
        Vec::new()
    };
    Ok(LiveData {
        limits,
        plan: token(&sand, "/cursorPlanName")
            .filter(|_| sand["isUnified"] == true)
            .or_else(|| token(&plan, "/planInfo/planName"))
            .or_else(|| token(&body, "/membershipType"))
            .or_else(|| token(&body, "/plan"))
            .or_else(|| token(&summary, "/membershipType")),
        records,
    })
}
fn cursor_desktop_token(homes: &[PathBuf]) -> Result<Option<String>> {
    let mut decode_error = None;
    for home in homes {
        let path = home.join("User/globalStorage/state.vscdb");
        if !path.exists() {
            continue;
        }
        if let Ok(connection) =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            && let Ok(value) = connection.query_row(
                "SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken'",
                [],
                |row| row.get::<_, String>(0),
            )
        {
            match auth::decode(&value, home) {
                Ok(access) if !access.is_empty() => return Ok(Some(access)),
                Ok(_) => {}
                Err(error) => decode_error = Some(error),
            }
        }
    }
    if let Some(access) = keychain("cursor-access-token", Some("cursor-user")) {
        return Ok(Some(access));
    }
    match decode_error {
        Some(error) => Err(error),
        None => Ok(None),
    }
}
fn session_cookie(access: &str) -> Result<(String, String)> {
    let claims: Value = access
        .split('.')
        .nth(1)
        .and_then(|data| URL_SAFE_NO_PAD.decode(data).ok())
        .and_then(|data| serde_json::from_slice(&data).ok())
        .context("Cursor login is unreadable. Sign in to Cursor again.")?;
    let subject = claims["sub"]
        .as_str()
        .context("Cursor login did not identify an account")?;
    let user = subject.rsplit('|').next().unwrap_or(subject);
    let cookie = format!(
        "WorkosCursorSessionToken={}",
        urlencoding::encode(&format!("{user}::{access}"))
    );
    Ok((cookie, hash(subject)))
}
fn cursor_history(client: &Client, access: &str) -> Result<Vec<UsageRecord>> {
    let (cookie, account) = session_cookie(access)?;
    let now = Utc::now();
    let since = now - chrono::Duration::days(90);
    let mut events = vec![];
    let mut completed = false;
    let mut expected = None;
    let deadline = Instant::now() + Duration::from_secs(60);
    for page in 1..=1000 {
        if Instant::now() >= deadline {
            bail!("Cursor history timed out. Previous results are retained.");
        }
        let body = response(client.post("https://cursor.com/api/dashboard/get-filtered-usage-events").header("Origin","https://cursor.com").header("Cookie",&cookie).json(&json!({"page":page,"pageSize":1000,"startDate":since.timestamp_millis().to_string(),"endDate":now.timestamp_millis().to_string()})),Provider::Cursor)?;
        let batch = body["usageEventsDisplay"]
            .as_array()
            .context("Cursor returned an unsupported history format")?;
        if let Some(total) = body["totalUsageEventsCount"].as_u64() {
            if expected.is_some_and(|old| old != total) {
                bail!("Cursor history changed during refresh. Refresh to retry.");
            }
            expected = Some(total);
        }
        events.extend(batch.iter().cloned());
        if batch.len() < 1000 {
            completed = true;
            break;
        }
    }
    if !completed || expected.is_some_and(|total| total != events.len() as u64) {
        bail!("Cursor returned incomplete history. Previous results are retained.");
    }
    parse_cursor_events(&events, &account)
}
pub fn parse_cursor_events(events: &[Value], account: &str) -> Result<Vec<UsageRecord>> {
    let mut records = vec![];
    let mut occurrences = HashMap::new();
    for event in events {
        let usage = &event["tokenUsage"];
        if !usage.is_object() {
            continue;
        }
        let stamp = event["timestamp"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| event["timestamp"].to_string());
        let timestamp = date(&stamp).context("Cursor returned an invalid timestamp")?;
        let model = event["model"]
            .as_str()
            .context("Cursor returned an unnamed model")?
            .to_owned();
        let tokens = Tokens {
            input: uint(&usage["inputTokens"]),
            output: uint(&usage["outputTokens"]),
            cache_read: uint(&usage["cacheReadTokens"]),
            cache_write: uint(&usage["cacheWriteTokens"]),
        };
        let reported_cost = number(&usage["totalCents"]).map(|cost| cost / 100.0);
        let key = hash(&format!(
            "{account}:{timestamp}:{model}:{tokens:?}:{reported_cost:?}"
        ));
        let occurrence = occurrences.entry(key.clone()).or_insert(0);
        let id = format!("{key}:{occurrence}");
        *occurrence += 1;
        records.push(UsageRecord {
            id,
            provider: Provider::Cursor,
            timestamp,
            model,
            session: event["conversationId"].as_str().unwrap_or("").into(),
            project: "Cursor account".into(),
            tokens,
            reported_cost,
            source: "Cursor account API".into(),
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monthly() -> Value {
        json!({"billingCycleStart":"2026-10-01T00:00:00Z","billingCycleEnd":"2026-11-01T00:00:00Z",
            "planUsage":{"totalPercentUsed":100,"autoPercentUsed":62.86,"apiPercentUsed":0}})
    }

    #[test]
    fn unified_weekly_pool_replaces_stale_monthly_and_api_allowances() {
        let sand = json!({"isUnified":true,"usagePercent":10.20585,"cursorPlanName":"Premium",
            "currentPeriodStart":"2026-10-15T10:27:36.178Z","nextResetTimestampUtc":"2026-10-22T10:27:36.178Z"});
        let limits = account_limits(&monthly(), &Value::Null, &sand, &monthly()).unwrap();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].label, "Weekly");
        assert_eq!(limits[0].used_percent, 10.20585);
        assert_eq!(limits[0].duration_minutes, Some(10080));
        assert_eq!(limits[0].resets_at, date("2026-10-22T10:27:36.178Z"));
    }

    #[test]
    fn legacy_accounts_keep_monthly_pools_and_separate_grok_bot_week() {
        let sand =
            json!({"isUnified":false,"usagePercent":8,"nextResetTimestampUtc":"1792664856178"});
        let limits = account_limits(&monthly(), &Value::Null, &sand, &Value::Null).unwrap();
        assert_eq!(
            limits.iter().map(|w| w.label.as_str()).collect::<Vec<_>>(),
            [
                "Monthly",
                "Cursor models",
                "Other models",
                "Grok Bot weekly"
            ]
        );
        assert_eq!(limits[0].duration_minutes, Some(31 * 1440));
        assert_ne!(limits[0].resets_at, limits[3].resets_at);
    }

    #[test]
    fn weekly_rpc_period_works_without_optional_endpoints() {
        let current = json!({"billingCycleStart":"1792060056178","billingCycleEnd":"1792664856178",
            "planUsage":{"totalPercentUsed":42,"autoPercentUsed":42,"apiPercentUsed":0}});
        let limits = account_limits(&current, &Value::Null, &Value::Null, &Value::Null).unwrap();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].label, "Weekly");
        assert_eq!(limits[0].resets_at, date("1792664856178"));
    }

    #[test]
    fn plan_period_is_used_when_cycle_start_is_missing() {
        for period in [json!(2), json!("INCLUDED_USAGE_PERIOD_WEEKLY")] {
            let plan = json!({"planInfo":{"includedUsagePeriod":period,"includedUsageResetsAt":"1792664856178"}});
            let current = json!({"planUsage":{"totalPercentUsed":25}});
            let limits = account_limits(&current, &plan, &Value::Null, &Value::Null).unwrap();
            assert_eq!(limits[0].label, "Weekly");
            assert_eq!(limits[0].resets_at, date("1792664856178"));
        }
    }

    #[test]
    fn unified_accounts_never_fall_back_to_a_stale_monthly_percentage() {
        assert!(
            account_limits(
                &monthly(),
                &Value::Null,
                &json!({"isUnified":true}),
                &monthly()
            )
            .is_err()
        );
        assert!(account_limits(&Value::Null, &Value::Null, &Value::Null, &Value::Null).is_err());
        assert!(account_limits(&Value::Null, &Value::Null, &Value::Null, &json!([])).is_err());
    }

    #[test]
    fn weekly_plan_without_allowance_reset_never_uses_payment_renewal() {
        let current = json!({"billingCycleEnd":"2026-11-30T00:00:00Z",
            "planUsage":{"totalPercentUsed":25}});
        let plan = json!({"planInfo":{"includedUsagePeriod":"INCLUDED_USAGE_PERIOD_WEEKLY"}});
        let limits = account_limits(&current, &plan, &Value::Null, &Value::Null).unwrap();
        assert_eq!(limits[0].label, "Weekly");
        assert_eq!(limits[0].resets_at, None);
    }

    #[test]
    fn unreadable_desktop_login_does_not_hide_another_profile() {
        let root = tempfile::tempdir().unwrap();
        let homes = [root.path().join("first"), root.path().join("second")];
        for (home, value) in homes
            .iter()
            .zip(["cursor-safe-storage:v2:secret", "test-token"])
        {
            std::fs::create_dir_all(home.join("User/globalStorage")).unwrap();
            let connection =
                rusqlite::Connection::open(home.join("User/globalStorage/state.vscdb")).unwrap();
            connection
                .execute("CREATE TABLE ItemTable (key TEXT, value TEXT)", [])
                .unwrap();
            connection
                .execute(
                    "INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', ?1)",
                    [value],
                )
                .unwrap();
        }
        assert_eq!(
            cursor_desktop_token(&homes).unwrap().as_deref(),
            Some("test-token")
        );
    }

    #[test]
    fn team_summary_and_dollar_amounts_supply_missing_percentages() {
        let summary = json!({"billingCycleEnd":"2026-11-01T00:00:00Z",
            "individualUsage":{"overall":{"used":71,"limit":10000}}});
        let limits = account_limits(&Value::Null, &Value::Null, &Value::Null, &summary).unwrap();
        assert!((limits[0].used_percent - 0.71).abs() < 1e-9);
        let current = json!({"planUsage":{"totalSpend":"250","limit":"1000","autoSpend":100,"autoLimit":500}});
        let limits = cursor_limits(&current);
        assert_eq!(limits[0].used_percent, 25.);
        assert_eq!(limits[1].used_percent, 20.);
    }

    #[test]
    fn reported_fractional_cents_and_token_strings_preserve_models_and_duplicates() {
        let event = json!({"timestamp":"1792060056178","model":"composer-2.5-fast",
            "tokenUsage":{"inputTokens":"1000","outputTokens":200,"cacheReadTokens":"500","cacheWriteTokens":0,"totalCents":"12.345"}});
        let records = parse_cursor_events(&[event.clone(), event], "test-account").unwrap();
        assert_eq!(records.len(), 2);
        assert_ne!(records[0].id, records[1].id);
        assert_eq!(records[0].tokens.total(), 1700);
        assert_eq!(records[0].model, "composer-2.5-fast");
        assert!((records[0].reported_cost.unwrap() - 0.12345).abs() < 1e-9);
        assert_eq!(
            crate::analytics::record_cost(&records[0]),
            Some((0.12345, false))
        );
        for (cost, expected) in [
            (json!(0), Some(0.)),
            (json!(-1), None),
            (json!("NaN"), None),
            (Value::Null, None),
        ] {
            let event = json!({"timestamp":"1792060056178","model":"unlisted-model","tokenUsage":{"totalCents":cost}});
            assert_eq!(
                parse_cursor_events(&[event], "test").unwrap()[0].reported_cost,
                expected
            );
        }
    }
}
