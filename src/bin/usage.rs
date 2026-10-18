use anyhow::{Result, bail};
use token_usage::{
    analytics::{self, Period},
    config::Config,
    model::Snapshot,
    providers,
    store::Store,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "usage [--refresh] [--online|--offline] [--json] [--period 2h|24h|7d|30d|90d|today] [--days 1|7|30|90]\n\nUsage and allowances for ChatGPT, Claude Code, Cursor, Kiro, Antigravity, and OpenCode. Costs use reported amounts or public API-rate estimates. --online checks existing logins and downloads prices; --offline reads local history and cached allowances/prices without network requests. New installs default to offline; existing settings are respected.\nNo prompts, conversations, or credentials are printed."
        );
        return Ok(());
    }
    let mut config = Config::load()?;
    if args.contains(&"--online".into()) {
        config.online = true;
    }
    if args.contains(&"--offline".into()) {
        config.online = false;
    }
    let days = args
        .iter()
        .position(|arg| arg == "--days")
        .map(|index| args.get(index + 1).map(String::as_str).unwrap_or(""))
        .unwrap_or("30");
    let mut period = match days {
        "1" => Period::Day,
        "7" => Period::Week,
        "30" => Period::Month,
        "90" => Period::Quarter,
        _ => bail!("--days must be 1, 7, 30, or 90"),
    };
    if let Some(index) = args.iter().position(|arg| arg == "--period") {
        period = match args.get(index + 1).map(String::as_str) {
            Some("2h") => Period::TwoHours,
            Some("24h") => Period::Day,
            Some("7d") => Period::Week,
            Some("30d") => Period::Month,
            Some("90d") => Period::Quarter,
            Some("today") => Period::Today,
            _ => bail!("--period must be 2h, 24h, 7d, 30d, 90d, or today"),
        };
    }
    let cached = Store::open()?.load()?.unwrap_or_else(Snapshot::default);
    let snapshot = if args.contains(&"--refresh".into())
        || args.contains(&"--online".into())
        || cached.refreshed_at.is_none()
    {
        providers::collect(&config, &cached)?
    } else {
        cached
    };
    token_usage::pricing::prepare(config.online);
    let now = chrono::Utc::now();
    let analysis = analytics::analyze(&snapshot, &config, period, None, now);
    if args.contains(&"--json".into()) {
        let accounts = snapshot.providers.iter().filter(|status| config.enabled[status.provider.index()] && (status.detected || status.connected || snapshot.records.iter().any(|record| record.provider == status.provider))).map(|status| {
            let totals = &analysis.providers[status.provider.index()];
            let mut public_status = status.clone();
            public_status.account_fingerprint = None;
            serde_json::json!({"account":status.provider,"plan":status.plan,"reported_usage_cost_usd":totals.reported_spend(),"usage_cost_usd":totals.spend(),"cost_reporting_events":totals.reported_rows,"tokens":totals.tokens,"sessions":totals.sessions,"status":public_status})
        }).collect::<Vec<_>>();
        let models = analysis.models.iter().map(|row| serde_json::json!({"model":row.model,"account":row.provider,"tokens":row.totals.tokens,"reported_usage_cost_usd":row.totals.reported_spend(),"usage_cost_usd":row.totals.spend()})).collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"schema":"token-usage.v2","days":period.days(),"period":period.key(),"starts_at":period.start(now),"ends_at":now,"reported_usage_cost_usd":analysis.totals.reported_spend(),"usage_cost_usd":analysis.totals.spend(),"cost_reporting_events":analysis.totals.reported_rows,"tokens":analysis.totals.tokens,"sessions":analysis.totals.sessions,"accounts":accounts,"models":models,"refreshed_at":snapshot.refreshed_at,"notices":snapshot.notices})
            )?
        );
    } else {
        let sep = analytics::separator();
        println!(
            "Token Usage{sep}{}\n{}",
            period.label(),
            [
                format!(
                    "Tokens: {}",
                    analytics::compact(analysis.totals.tokens.total())
                ),
                format!("Sessions: {}", analysis.totals.sessions),
            ]
            .join(sep)
        );
        for status in &snapshot.providers {
            let totals = &analysis.providers[status.provider.index()];
            if !config.enabled[status.provider.index()]
                || (!status.detected && !status.connected && totals.events == 0)
            {
                continue;
            }
            let spend = totals
                .spend()
                .map(|cost| format!("${cost:.2}"))
                .unwrap_or_else(|| "No public rate".into());
            let cost_label = if totals.estimated_rows > 0 && totals.reported_rows == 0 {
                "API-equivalent cost"
            } else {
                "Cost"
            };
            println!(
                "{}",
                [
                    status.provider.account_name().to_string(),
                    status
                        .plan
                        .as_deref()
                        .unwrap_or("Plan unavailable")
                        .to_string(),
                    format!("{} tokens", analytics::compact(totals.tokens.total())),
                    format!("{cost_label}: {spend}"),
                ]
                .join(analytics::separator())
            );
        }
    }
    Ok(())
}
