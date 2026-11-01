use crate::{
    config::Config,
    model::{Provider, Snapshot, Tokens, UsageRecord},
    pricing,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Today,
    TwoHours,
    Day,
    Week,
    Month,
    Quarter,
}
impl Period {
    pub const ALL: [Self; 5] = [
        Self::TwoHours,
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Quarter,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::TwoHours => "2h",
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Quarter => "90d",
        }
    }
    pub fn days(self) -> i64 {
        match self {
            Self::Today | Self::TwoHours | Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 90,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::TwoHours => "Last 2 hours",
            Self::Day => "Last 24 hours",
            Self::Week => "7 days",
            Self::Month => "30 days",
            Self::Quarter => "90 days",
        }
    }
    pub fn start(self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self != Self::Today {
            return now
                - match self {
                    Self::TwoHours => Duration::hours(2),
                    Self::Day => Duration::hours(24),
                    _ => Duration::days(self.days()),
                };
        }
        let date = now.with_timezone(&Local).date_naive();
        Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap_or_else(|| now.with_timezone(&Local))
            .with_timezone(&Utc)
    }
    pub fn bucket_seconds(self) -> i64 {
        match self {
            Self::TwoHours => 300,
            Self::Today | Self::Day => 3600,
            _ => 86400,
        }
    }
    pub fn chart_title(self) -> String {
        format!(
            "Usage{}{}",
            separator(),
            match self {
                Self::TwoHours => "5-minute intervals",
                Self::Today | Self::Day => "hourly",
                _ => "daily",
            }
        )
    }
    pub fn bucket_label(self, at: DateTime<Utc>) -> String {
        at.with_timezone(&Local)
            .format(if self.bucket_seconds() < 86400 {
                "%H:%M"
            } else {
                "%b %-d"
            })
            .to_string()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Totals {
    pub tokens: Tokens,
    pub reported: f64,
    pub reported_rows: usize,
    pub estimated: f64,
    pub estimated_rows: usize,
    pub events: usize,
    pub sessions: usize,
}
impl Totals {
    /// Reported account cost only. None means the provider sent no dollar amount.
    pub fn reported_spend(&self) -> Option<f64> {
        (self.reported_rows > 0).then_some(self.reported)
    }
    /// Reported cost plus API-equivalent estimates for rows the provider left unpriced.
    pub fn spend(&self) -> Option<f64> {
        let rows = self.reported_rows + self.estimated_rows;
        (rows > 0).then_some(self.reported + self.estimated)
    }
}
#[derive(Debug, Clone)]
pub struct ModelRow {
    pub model: String,
    pub provider: Provider,
    pub totals: Totals,
}
#[derive(Debug, Clone)]
pub struct Bucket {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub cost: [f64; Provider::COUNT],
    pub cost_rows: [usize; Provider::COUNT],
    pub tokens: [u64; Provider::COUNT],
}
#[derive(Debug, Clone)]
pub struct Analysis {
    pub totals: Totals,
    pub providers: [Totals; Provider::COUNT],
    pub models: Vec<ModelRow>,
    pub buckets: Vec<Bucket>,
}
pub fn matches(
    record: &UsageRecord,
    period: Period,
    provider: Option<Provider>,
    now: DateTime<Utc>,
) -> bool {
    record.timestamp >= period.start(now)
        && record.timestamp <= now
        && provider.is_none_or(|p| p == record.provider)
}
/// Reported account cost, or an API-rate estimate when the provider sent none.
/// The bool is true when the amount is estimated.
pub fn record_cost(record: &UsageRecord) -> Option<(f64, bool)> {
    if let Some(cost) = record
        .reported_cost
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
    {
        return Some((cost, false));
    }
    pricing::estimate(record).map(|cost| (cost, true))
}
fn add(totals: &mut Totals, record: &UsageRecord) {
    totals.tokens.add(&record.tokens);
    totals.events += 1;
    if let Some((cost, estimated)) = record_cost(record) {
        if estimated {
            totals.estimated += cost;
            totals.estimated_rows += 1;
        } else {
            totals.reported += cost;
            totals.reported_rows += 1;
        }
    }
}
pub fn analyze(
    snapshot: &Snapshot,
    config: &Config,
    period: Period,
    provider: Option<Provider>,
    now: DateTime<Utc>,
) -> Analysis {
    let first = period.start(now);
    let interval = period.bucket_seconds();
    let bucket_count = ((now - first).num_seconds().max(1) + interval - 1) / interval;
    let mut analysis = Analysis {
        totals: Totals::default(),
        providers: Default::default(),
        models: vec![],
        buckets: (0..bucket_count)
            .map(|d| Bucket {
                start: first + Duration::seconds(d * interval),
                end: (first + Duration::seconds((d + 1) * interval)).min(now),
                cost: [0.0; Provider::COUNT],
                cost_rows: [0; Provider::COUNT],
                tokens: [0; Provider::COUNT],
            })
            .collect(),
    };
    let mut models: BTreeMap<(Provider, String), Totals> = BTreeMap::new();
    let mut model_sessions: BTreeMap<(Provider, String), BTreeSet<String>> = BTreeMap::new();
    let mut sessions = BTreeSet::new();
    for record in &snapshot.records {
        if !config.enabled[record.provider.index()]
            || provider.is_some_and(|p| p != record.provider)
        {
            continue;
        }
        if !matches(record, period, provider, now) {
            continue;
        }
        add(&mut analysis.totals, record);
        add(&mut analysis.providers[record.provider.index()], record);
        add(
            models
                .entry((record.provider, record.model.clone()))
                .or_default(),
            record,
        );
        if !record.session.is_empty() {
            sessions.insert((record.provider, record.session.clone()));
            model_sessions
                .entry((record.provider, record.model.clone()))
                .or_default()
                .insert(record.session.clone());
        }
        let index = ((record.timestamp - first).num_seconds() / interval).min(bucket_count - 1);
        if let Some(bucket) = usize::try_from(index)
            .ok()
            .and_then(|index| analysis.buckets.get_mut(index))
        {
            bucket.tokens[record.provider.index()] =
                bucket.tokens[record.provider.index()].saturating_add(record.tokens.total());
            if let Some((cost, _)) = record_cost(record) {
                bucket.cost[record.provider.index()] += cost;
                bucket.cost_rows[record.provider.index()] += 1;
            }
        }
    }
    analysis.totals.sessions = sessions.len();
    for (provider, _) in sessions {
        analysis.providers[provider.index()].sessions += 1;
    }
    analysis.models = models
        .into_iter()
        .map(|((provider, model), mut totals)| {
            totals.sessions = model_sessions
                .get(&(provider, model.clone()))
                .map_or(0, BTreeSet::len);
            ModelRow {
                model,
                provider,
                totals,
            }
        })
        .collect();
    analysis
        .models
        .sort_by_key(|row| std::cmp::Reverse(row.totals.tokens.total()));
    analysis
}
#[derive(Debug, Clone)]
pub struct ProjectRow {
    /// Empty when the source did not report a project.
    pub project: String,
    pub totals: Totals,
}
#[derive(Debug, Clone)]
pub struct MixBucket {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub tokens: Tokens,
}
/// Breakdowns that complement [`Analysis`]: token types over time, local
/// weekday/hour activity, and per-project totals.
#[derive(Debug, Clone)]
pub struct Insights {
    pub mix: Vec<MixBucket>,
    /// Tokens by local weekday (Monday = 0) and local hour.
    pub heatmap: [[u64; 24]; 7],
    pub projects: Vec<ProjectRow>,
}
pub fn insights(
    snapshot: &Snapshot,
    config: &Config,
    period: Period,
    provider: Option<Provider>,
    now: DateTime<Utc>,
) -> Insights {
    use chrono::{Datelike, Timelike};
    let first = period.start(now);
    let interval = period.bucket_seconds();
    let bucket_count = ((now - first).num_seconds().max(1) + interval - 1) / interval;
    let mut result = Insights {
        mix: (0..bucket_count)
            .map(|d| MixBucket {
                start: first + Duration::seconds(d * interval),
                end: (first + Duration::seconds((d + 1) * interval)).min(now),
                tokens: Tokens::default(),
            })
            .collect(),
        heatmap: [[0; 24]; 7],
        projects: vec![],
    };
    let mut projects: BTreeMap<String, Totals> = BTreeMap::new();
    let mut project_sessions: BTreeMap<String, BTreeSet<(Provider, String)>> = BTreeMap::new();
    for record in &snapshot.records {
        if !config.enabled[record.provider.index()] || !matches(record, period, provider, now) {
            continue;
        }
        let index = ((record.timestamp - first).num_seconds() / interval).min(bucket_count - 1);
        if let Some(bucket) = usize::try_from(index)
            .ok()
            .and_then(|index| result.mix.get_mut(index))
        {
            bucket.tokens.add(&record.tokens);
        }
        let local = record.timestamp.with_timezone(&Local);
        let cell = &mut result.heatmap[local.weekday().num_days_from_monday() as usize]
            [local.hour() as usize];
        *cell = cell.saturating_add(record.tokens.total());
        add(projects.entry(record.project.clone()).or_default(), record);
        if !record.session.is_empty() {
            project_sessions
                .entry(record.project.clone())
                .or_default()
                .insert((record.provider, record.session.clone()));
        }
    }
    result.projects = projects
        .into_iter()
        .map(|(project, mut totals)| {
            totals.sessions = project_sessions.get(&project).map_or(0, BTreeSet::len);
            ProjectRow { project, totals }
        })
        .collect();
    result
        .projects
        .sort_by_key(|row| std::cmp::Reverse(row.totals.tokens.total()));
    result
}
pub fn export_csv(
    path: &Path,
    snapshot: &Snapshot,
    config: &Config,
    period: Period,
    provider: Option<Provider>,
) -> Result<usize> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "timestamp",
        "account",
        "model",
        "project",
        "session",
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "reported_usd",
        "api_rate_usd",
    ])?;
    let mut count = 0;
    for record in &snapshot.records {
        if !config.enabled[record.provider.index()]
            || !matches(record, period, provider, Utc::now())
        {
            continue;
        }
        writer.write_record([
            record.timestamp.to_rfc3339(),
            record.provider.key().into(),
            csv_text(&record.model),
            csv_text(&record.project),
            csv_text(&record.session),
            record.tokens.input.to_string(),
            record.tokens.output.to_string(),
            record.tokens.cache_read.to_string(),
            record.tokens.cache_write.to_string(),
            record
                .reported_cost
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .map(|p| format!("{p:.6}"))
                .unwrap_or_default(),
            record_cost(record)
                .filter(|(_, estimated)| *estimated)
                .map(|(cost, _)| format!("{cost:.6}"))
                .unwrap_or_default(),
        ])?;
        count += 1;
    }
    writer.flush()?;
    Ok(count)
}
// CSV quoting does not stop spreadsheet applications from evaluating formulas.
fn csv_text(value: &str) -> String {
    let trimmed = value.trim_start_matches(|character: char| {
        character.is_whitespace() || character.is_control() || character == '\u{feff}'
    });
    if trimmed.starts_with(['=', '+', '-', '@']) || value.starts_with(['\t', '\r', '\n']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}
pub fn compact(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1e9)
    } else if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1e6)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1e3)
    } else {
        value.to_string()
    }
}

/// Shown for a missing count, cost, project, or other empty reading.
/// A hyphen stays readable on macOS and Windows. An em dash or other
/// Unicode blank is easy to mis-decode and then draw as garbage.
pub fn empty_value() -> &'static str {
    "-"
}

/// Spaces and a pipe, used wherever a middle dot used to separate two labels.
/// A middle dot is two UTF-8 bytes, which a Windows-1252 console shows as `A` with a circumflex.
pub fn separator() -> &'static str {
    " | "
}

/// Privacy mask for amounts and counts.
/// A bullet is three UTF-8 bytes, which a Windows console shows as garbage.
pub fn hidden_value() -> &'static str {
    "****"
}

#[cfg(test)]
mod tests {
    use super::{csv_text, empty_value, hidden_value, separator};

    #[test]
    fn csv_text_cannot_start_a_spreadsheet_formula() {
        for value in [
            "=HYPERLINK(\"https://example.invalid\")",
            "+1",
            "-1",
            "@SUM(A1)",
            "  =1",
            "\tdata",
            "\rdata",
            "\ndata",
            "\0=1",
            "\u{feff}=1",
        ] {
            assert_eq!(csv_text(value), format!("'{value}"));
        }
        assert_eq!(csv_text("my-project"), "my-project");
    }

    #[test]
    fn empty_value_is_a_plain_hyphen() {
        let text = empty_value();
        assert_eq!(text, "-");
        assert!(text.is_ascii());
        assert_eq!(text.as_bytes(), b"-");
        for piece in [
            "\u{2014}",
            "\u{2013}",
            "\u{2026}",
            "\u{2022}",
            "\u{00b7}",
            "\u{00a0}",
            "\u{00c3}",
            "\u{00c2}",
            "\u{00e2}\u{20ac}",
            "\u{00e2}\u{20ac}\u{2122}",
        ] {
            assert!(
                !text.contains(piece),
                "empty value must not contain {piece:?}"
            );
        }
    }

    #[test]
    fn separator_and_hidden_value_are_plain_ascii() {
        assert_eq!(separator(), " | ");
        assert!(separator().is_ascii());
        assert_eq!(hidden_value(), "****");
        assert!(hidden_value().is_ascii());
        assert_eq!(
            super::Period::Day.chart_title(),
            format!("Usage{}hourly", separator())
        );
        assert!(super::Period::TwoHours.chart_title().is_ascii());
    }
}
