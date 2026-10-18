//! Everything the pages show that is derived from the usage history.
//!
//! Rendering runs on every frame that scrolls, hovers or resizes, so it must
//! never walk the history. Each derived value lives in a [`Slot`] keyed by the
//! inputs it was computed from. Rendering only reads the last finished value;
//! when the inputs change, the value is recomputed on the background executor
//! and swapped in when it lands. A newer request replaces (and cancels) an
//! older one, so results for outdated keystrokes are discarded.
use super::*;
use std::{path::PathBuf, sync::Arc};
use token_dock::analytics::{Analysis, Insights, ModelRow, ProjectRow};

/// The inputs every history-derived value shares.
#[derive(Clone, PartialEq)]
pub struct Scope {
    /// Bumped when the history changes, rates update, or the minute ticks.
    pub version: u64,
    pub period: Period,
    pub provider: Option<Provider>,
    pub enabled: [bool; Provider::COUNT],
}

#[derive(Clone, PartialEq)]
pub struct ModelsKey {
    pub scope: Scope,
    pub query: String,
    pub cost: CostFilter,
    pub sort: Sort,
}
pub struct ModelsResult {
    pub rows: Vec<ModelRow>,
    pub total: usize,
    pub estimates: bool,
}

#[derive(Clone, PartialEq)]
pub struct ActivityKey {
    pub scope: Scope,
    pub query: String,
    pub cost: CostFilter,
    pub sort: Sort,
    pub privacy: bool,
}
pub struct ActivityResult {
    /// The history the indices point into, so a newer refresh never misaligns them.
    pub snapshot: Arc<Snapshot>,
    pub indices: Vec<usize>,
    /// Events in the period before search and cost filters.
    pub total: usize,
}
impl ActivityResult {
    pub fn records(&self) -> impl Iterator<Item = &token_dock::model::UsageRecord> {
        self.indices
            .iter()
            .map(|&index| &self.snapshot.records[index])
    }
}

#[derive(Clone, PartialEq)]
pub struct ProjectsKey {
    pub scope: Scope,
    pub sort: Sort,
}

pub struct Slot<K, V> {
    key: Option<K>,
    value: Option<Arc<V>>,
    wanted: Option<K>,
    task: Option<Task<()>>,
}
impl<K, V> Default for Slot<K, V> {
    fn default() -> Self {
        Self {
            key: None,
            value: None,
            wanted: None,
            task: None,
        }
    }
}
impl<K: PartialEq, V> Slot<K, V> {
    pub fn value(&self) -> Option<Arc<V>> {
        self.value.clone()
    }
}

#[derive(Default)]
pub struct Derived {
    pub overview: Slot<Scope, Analysis>,
    pub scoped: Slot<Scope, Analysis>,
    pub today: Slot<Scope, Analysis>,
    pub insights: Slot<Scope, Insights>,
    pub projects: Slot<ProjectsKey, Vec<ProjectRow>>,
    pub models: Slot<ModelsKey, ModelsResult>,
    pub activity: Slot<ActivityKey, ActivityResult>,
    /// Default data folders that exist, per provider. Checked off the UI thread.
    pub homes: Slot<(u64, u64), Vec<Vec<PathBuf>>>,
}

impl Dashboard {
    /// The last finished analysis, or an empty one before the first lands.
    pub(super) fn analysis_or_empty(
        &self,
        value: Option<Arc<Analysis>>,
        period: Period,
        provider: Option<Provider>,
    ) -> Arc<Analysis> {
        value.unwrap_or_else(|| {
            Arc::new(analytics::analyze(
                &Snapshot::default(),
                &self.config,
                period,
                provider,
                self.now,
            ))
        })
    }
    pub(super) fn scope(&self, period: Period, provider: Option<Provider>) -> Scope {
        Scope {
            version: self.version,
            period,
            provider,
            enabled: self.config.enabled,
        }
    }
    /// Requests `work` for `key` unless that key is already shown or running.
    fn schedule<K, V>(
        &mut self,
        slot: fn(&mut Derived) -> &mut Slot<K, V>,
        key: K,
        work: impl FnOnce() -> V + Send + 'static,
        on_ready: Option<fn(&mut Self)>,
        cx: &mut Context<Self>,
    ) where
        K: PartialEq + Clone + 'static,
        V: Send + Sync + 'static,
    {
        let current = slot(&mut self.derived);
        if current.wanted.as_ref() == Some(&key)
            || (current.wanted.is_none() && current.key.as_ref() == Some(&key))
        {
            return;
        }
        current.wanted = Some(key.clone());
        // Replacing the task drops the previous one, which cancels it.
        current.task = Some(cx.spawn(async move |view, cx| {
            let value = cx.background_executor().spawn(async move { work() }).await;
            let _ = view.update(cx, |view, cx| {
                let current = slot(&mut view.derived);
                if current.wanted.as_ref() != Some(&key) {
                    return;
                }
                current.key = Some(key);
                current.value = Some(Arc::new(value));
                current.wanted = None;
                current.task = None;
                if let Some(on_ready) = on_ready {
                    on_ready(view);
                }
                cx.notify();
            });
        }));
    }

    fn schedule_analysis(
        &mut self,
        slot: fn(&mut Derived) -> &mut Slot<Scope, Analysis>,
        scope: Scope,
        on_ready: Option<fn(&mut Self)>,
        cx: &mut Context<Self>,
    ) {
        let (data, config, now) = (self.live.clone(), self.config.clone(), self.now);
        let (period, provider) = (scope.period, scope.provider);
        self.schedule(
            slot,
            scope,
            move || analytics::analyze(&data, &config, period, provider, now),
            on_ready,
            cx,
        );
    }

    /// Tray figures are needed even while the window is hidden.
    pub(super) fn ensure_today(&mut self, cx: &mut Context<Self>) {
        let scope = self.scope(Period::Today, None);
        self.schedule_analysis(|d| &mut d.today, scope, Some(Self::update_tray), cx);
    }

    /// Requests whatever the visible page needs. Cheap when nothing changed.
    pub(super) fn ensure_derived(&mut self, cx: &mut Context<Self>) {
        if self.compact {
            self.ensure_today(cx);
            return;
        }
        match self.page {
            Page::Overview => {
                let scope = self.scope(self.period, None);
                self.schedule_analysis(|d| &mut d.overview, scope, None, cx);
            }
            Page::Insights => {
                let scope = self.scope(self.period, self.provider);
                self.schedule_analysis(|d| &mut d.scoped, scope.clone(), None, cx);
                let (data, config, now) = (self.live.clone(), self.config.clone(), self.now);
                let (period, provider) = (scope.period, scope.provider);
                self.schedule(
                    |d| &mut d.insights,
                    scope.clone(),
                    move || analytics::insights(&data, &config, period, provider, now),
                    None,
                    cx,
                );
                // Projects sort independently of the rest of the insights.
                if let Some(insights) = self
                    .derived
                    .insights
                    .value()
                    .filter(|_| self.derived.insights.key.as_ref() == Some(&scope))
                {
                    let sort = self.project_sort;
                    self.schedule(
                        |d| &mut d.projects,
                        ProjectsKey { scope, sort },
                        move || {
                            let mut rows = insights.projects.clone();
                            sort.projects(&mut rows);
                            rows
                        },
                        None,
                        cx,
                    );
                }
            }
            Page::Models => {
                let key = ModelsKey {
                    scope: self.scope(self.period, self.provider),
                    query: self.model_query.read(cx).value().to_string(),
                    cost: self.model_cost,
                    sort: self.model_sort,
                };
                let (data, config, now) = (self.live.clone(), self.config.clone(), self.now);
                let job = key.clone();
                self.schedule(
                    |d| &mut d.models,
                    key,
                    move || {
                        let mut analysis = analytics::analyze(
                            &data,
                            &config,
                            job.scope.period,
                            job.scope.provider,
                            now,
                        );
                        let total = analysis.models.len();
                        analysis.models.retain(|row| {
                            job.cost.totals(&row.totals)
                                && table::matches_query(
                                    &job.query,
                                    &[&row.model, row.provider.name(), row.provider.key()],
                                )
                        });
                        job.sort.models(&mut analysis.models);
                        let estimates = analysis.models.iter().any(|row| {
                            row.totals.reported_rows == 0 && row.totals.estimated_rows > 0
                        });
                        ModelsResult {
                            rows: analysis.models,
                            total,
                            estimates,
                        }
                    },
                    None,
                    cx,
                );
            }
            Page::Activity => self.ensure_activity(cx),
            Page::Connections => {
                let key = (self.config_revision, self.version);
                let config = self.config.clone();
                self.schedule(
                    |d| &mut d.homes,
                    key,
                    move || {
                        Provider::ALL
                            .into_iter()
                            .map(|provider| {
                                token_dock::providers::homes(provider, &config)
                                    .into_iter()
                                    .filter(|path| path.exists())
                                    .collect()
                            })
                            .collect()
                    },
                    None,
                    cx,
                );
            }
            Page::Limits | Page::Settings => {}
        }
    }

    pub(super) fn activity_key(&self, cx: &App) -> ActivityKey {
        ActivityKey {
            scope: self.scope(self.period, self.provider),
            query: self.activity_query.read(cx).value().to_string(),
            cost: self.activity_cost,
            sort: self.activity_sort,
            privacy: self.config.privacy,
        }
    }
    pub(super) fn ensure_activity(&mut self, cx: &mut Context<Self>) {
        let key = self.activity_key(cx);
        let (data, now) = (self.live.clone(), self.now);
        let job = key.clone();
        self.schedule(
            |d| &mut d.activity,
            key,
            move || {
                let mut total = 0;
                let mut matched = data
                    .records
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| {
                        job.scope.enabled[record.provider.index()]
                            && analytics::matches(record, job.scope.period, job.scope.provider, now)
                    })
                    .inspect(|_| total += 1)
                    .filter(|(_, record)| {
                        job.cost.record(record)
                            && table::matches_query(
                                &job.query,
                                &[
                                    &record.model,
                                    record.provider.name(),
                                    record.provider.key(),
                                    if job.privacy { "" } else { &record.project },
                                ],
                            )
                    })
                    .collect::<Vec<_>>();
                job.sort.activity_indexed(&mut matched);
                ActivityResult {
                    indices: matched.into_iter().map(|(index, _)| index).collect(),
                    snapshot: data,
                    total,
                }
            },
            None,
            cx,
        );
    }
}
