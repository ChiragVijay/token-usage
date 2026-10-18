//! Kiro credits are account allowances, never token counts or dollar spend.
mod quota;
use crate::model::Provider;
use crate::providers::{
    history::date,
    http::{read, response, token},
    live::LiveData,
    local,
};
use anyhow::{Context, Result, bail};
pub use quota::{CachedLimits, cached_limits, limits};
use reqwest::blocking::{Client, RequestBuilder};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) fn cli_home(local_data: &Path) -> PathBuf {
    local_data.join(if cfg!(windows) {
        "Kiro-Cli"
    } else {
        "kiro-cli"
    })
}

// The runtime requires the full AWS-JSON operation name and a Kiro/Q user agent.
// Kiro Crew's own credit reader uses this same read-only runtime operation.
fn cli_request(
    client: &Client,
    aws_region: &str,
    access: &str,
    profile: &str,
) -> Result<RequestBuilder> {
    let aws_region = region(aws_region)?;
    let mut body = json!({"origin":"AI_EDITOR","resourceType":"AGENTIC_REQUEST"});
    if !profile.is_empty() {
        body["profileArn"] = json!(profile);
    }
    Ok(client
        .post(format!("https://codewhisperer.{aws_region}.amazonaws.com/"))
        .header("Content-Type", "application/x-amz-json-1.0")
        .header(
            "x-amz-target",
            "com.amazon.aws.codewhisperer.runtime.AmazonCodeWhispererService.GetUsageLimits",
        )
        .header(
            "User-Agent",
            concat!(
                "AmazonQ-For-CLI/1.24.0 ua/2.0 lang/rust TokenUsage/",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .bearer_auth(access)
        .json(&body))
}

pub(super) fn fingerprint(homes: &[PathBuf]) -> Result<Option<String>> {
    for home in homes {
        let path = home.join("data.sqlite3");
        if !path.exists() {
            continue;
        }
        let connection = local::database(&path)?;
        if !local::table(&connection, "auth_kv")? {
            continue;
        }
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM auth_kv WHERE key='kirocli:odic:token'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(raw) = raw {
            let value: Value = serde_json::from_str(&raw)
                .map_err(|_| anyhow::anyhow!("Kiro login is unreadable"))?;
            return Ok(token(&value, "/refresh_token")
                .or_else(|| token(&value, "/access_token"))
                .map(|value| crate::providers::history::hash(&value)));
        }
    }
    Ok(homes
        .iter()
        .find_map(|home| read(&home.join("kiro-auth-token.json")))
        .and_then(|value| token(&value, "/refreshToken").or_else(|| token(&value, "/accessToken")))
        .map(|value| crate::providers::history::hash(&value)))
}

fn region(value: &str) -> Result<&str> {
    let parts: Vec<_> = value.split('-').collect();
    if !(3..=5).contains(&parts.len())
        || value.len() > 32
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
        || !parts.last().unwrap().chars().all(|c| c.is_ascii_digit())
    {
        bail!("Kiro login contains an invalid AWS region; sign in again");
    }
    Ok(value)
}
fn plan(body: &Value) -> Option<String> {
    token(body, "/subscriptionInfo/subscriptionTitle")
        .or_else(|| token(body, "/subscriptionInfo/type"))
}
fn data(body: Value) -> Result<LiveData> {
    let limits = limits(&body);
    if limits.is_empty() {
        bail!("Kiro did not report a credit allowance");
    }
    Ok(LiveData {
        limits,
        plan: plan(&body),
        records: vec![],
    })
}

pub fn fetch(client: &Client, homes: &[PathBuf]) -> Result<LiveData> {
    // Kiro CLI's own WAL database is the source used by its /usage command.
    for home in homes {
        let path = home.join("data.sqlite3");
        if !path.exists() {
            continue;
        }
        let connection = local::database(&path)
            .map_err(|_| anyhow::anyhow!("Kiro CLI login could not be read"))?;
        if !local::table(&connection, "auth_kv")? {
            continue;
        }
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM auth_kv WHERE key='kirocli:odic:token'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            continue;
        };
        let auth: Value = serde_json::from_str(&raw)
            .map_err(|_| anyhow::anyhow!("Kiro CLI login is unreadable; run kiro-cli login"))?;
        let aws_region = region(auth["region"].as_str().unwrap_or("us-east-1"))?;
        let access = crate::providers::auth::access(
            Provider::Kiro,
            token(&auth, "/access_token"),
            token(&auth, "/refresh_token"),
            auth["expires_at"].as_str().and_then(date),
            |refresh| {
                let registration: String = connection
                    .query_row(
                        "SELECT value FROM auth_kv WHERE key='kirocli:odic:device-registration'",
                        [],
                        |row| row.get(0),
                    )
                    .context("Run kiro-cli login to renew Kiro access")?;
                let registration: Value = serde_json::from_str(&registration)
                    .map_err(|_| anyhow::anyhow!("Kiro CLI registration is unreadable"))?;
                let refreshed = response(client.post(format!("https://oidc.{aws_region}.amazonaws.com/token")).json(&json!({"grantType":"refresh_token", "clientId":token(&registration,"/client_id").context("Run kiro-cli login")?, "clientSecret":token(&registration,"/client_secret").context("Run kiro-cli login")?, "refreshToken":refresh})),Provider::Kiro)?;
                Ok((
                    token(&refreshed, "/accessToken")
                        .context("Kiro could not renew access; run kiro-cli login")?,
                    token(&refreshed, "/refreshToken"),
                    refreshed["expiresIn"].as_u64().unwrap_or(3600),
                ))
            },
        )?;
        let profile: Option<String> = if local::table(&connection, "state")? {
            connection
                .query_row(
                    "SELECT value FROM state WHERE key='api.codewhisperer.profile'",
                    [],
                    |row| row.get(0),
                )
                .optional()?
        } else {
            None
        };
        let profile = profile
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| token(&value, "/arn"))
            .unwrap_or_default();
        return data(response(
            cli_request(client, aws_region, &access, &profile)?,
            Provider::Kiro,
        )?);
    }
    // IDE social/Identity Center credentials, from Kiro's specific SSO cache file.
    let auth = homes
        .iter()
        .find_map(|home| read(&home.join("kiro-auth-token.json")))
        .context("Sign in to Kiro or run kiro-cli login to read credit limits")?;
    let profile = token(&auth, "/profileArn").or_else(|| {
        homes.iter().find_map(|home| {
            read(&home.join("User/globalStorage/kiro.kiroagent/profile.json"))
                .and_then(|value| token(&value, "/arn"))
        })
    });
    let access = crate::providers::auth::access(
        Provider::Kiro,
        token(&auth, "/accessToken"),
        token(&auth, "/refreshToken"),
        auth["expiresAt"].as_str().and_then(date),
        |refresh| {
            if auth["authMethod"] != "social" {
                bail!("Kiro access expired; sign in to Kiro again");
            }
            let refreshed = response(
                client
                    .post("https://prod.us-east-1.auth.desktop.kiro.dev/refreshToken")
                    .json(&json!({"refreshToken":refresh})),
                Provider::Kiro,
            )?;
            Ok((
                token(&refreshed, "/accessToken").context("Sign in to Kiro again")?,
                token(&refreshed, "/refreshToken"),
                refreshed["expiresIn"].as_u64().unwrap_or(3600),
            ))
        },
    )?;
    let aws_region = region(
        profile
            .as_deref()
            .and_then(|arn| arn.split(':').nth(3))
            .unwrap_or("us-east-1"),
    )?;
    let mut request = client
        .get(format!(
            "https://q.{aws_region}.amazonaws.com/getUsageLimits"
        ))
        .bearer_auth(access)
        .query(&[
            ("origin", "AI_EDITOR"),
            ("resourceType", "AGENTIC_REQUEST"),
            ("profileArn", profile.as_deref().unwrap_or("")),
        ]);
    if auth["authMethod"] == "external_idp" {
        request = request.header("TokenType", "EXTERNAL_IDP");
    }
    data(response(request, Provider::Kiro)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_cli_login_is_read_from_local_app_data() {
        let dir = tempfile::TempDir::new().unwrap();
        let local = dir.path().join("Local");
        let roaming = dir.path().join("Roaming");
        let home = cli_home(&local);
        std::fs::create_dir_all(&home).unwrap();
        let db = rusqlite::Connection::open(home.join("data.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE auth_kv(key TEXT,value TEXT)")
            .unwrap();
        db.execute(
            "INSERT INTO auth_kv VALUES('kirocli:odic:token',?1)",
            [
                json!({"access_token":"fixture-access","refresh_token":"fixture-refresh"})
                    .to_string(),
            ],
        )
        .unwrap();
        assert!(fingerprint(&[roaming.join("kiro-cli")]).unwrap().is_none());
        assert!(fingerprint(&[home]).unwrap().is_some());
        assert!(!roaming.exists());
    }
    #[test]
    fn cli_usage_request_matches_the_runtime_protocol() {
        let client = Client::new();
        let request = cli_request(&client, "us-east-1", "fixture-access", "")
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://codewhisperer.us-east-1.amazonaws.com/"
        );
        assert_eq!(
            request.headers()["x-amz-target"],
            "com.amazon.aws.codewhisperer.runtime.AmazonCodeWhispererService.GetUsageLimits"
        );
        assert!(
            request.headers()["user-agent"]
                .to_str()
                .unwrap()
                .starts_with("AmazonQ-For-CLI/")
        );
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["origin"], "AI_EDITOR");
        assert_eq!(body["resourceType"], "AGENTIC_REQUEST");
        assert!(body.get("profileArn").is_none());
        let request = cli_request(&client, "us-east-1", "fixture-access", "fixture-profile")
            .unwrap()
            .build()
            .unwrap();
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["profileArn"], "fixture-profile");
        assert!(cli_request(&client, "evil.example", "fixture-access", "").is_err());
    }
    #[test]
    fn aws_regions_cannot_redirect_credentials_to_another_host() {
        assert!(region("us-east-1").is_ok());
        assert!(region("eu-west-2").is_ok());
        for value in [
            "evil.example",
            "us-east-1.amazonaws.com@evil.example",
            "us-east-1/path",
            "",
            "us--1",
        ] {
            assert!(region(value).is_err());
        }
    }
    #[test]
    fn cli_fingerprint_uses_its_read_only_login_and_disappears_after_logout() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = rusqlite::Connection::open(dir.path().join("data.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE auth_kv(key TEXT,value TEXT)")
            .unwrap();
        db.execute("INSERT INTO auth_kv VALUES('kirocli:odic:token',?1)",[json!({"refresh_token":"fixture-refresh", "access_token":"fixture-access","region":"us-east-1"}).to_string()]).unwrap();
        let identity = fingerprint(&[dir.path().into()]).unwrap().unwrap();
        assert!(!identity.contains("fixture"));
        db.execute("DELETE FROM auth_kv", []).unwrap();
        assert!(fingerprint(&[dir.path().into()]).unwrap().is_none());
    }
}
