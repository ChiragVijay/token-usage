mod charts;
mod controls;
mod derived;
mod insights;
mod layout;
mod table;
mod theme;
mod tray;
mod views;
mod window;

pub(crate) use window::WINDOW_TITLE;
#[cfg(windows)]
pub(crate) use window::claim_single_instance;

use chrono::Utc;
use controls::Segment;
use gpui::{div, prelude::*, px, rgb, *};
use gpui_component::input::{InputEvent, InputState};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use table::{Column, CostFilter, Sort, Table};
use theme::*;
use token_dock::{
    analytics::{self, Period},
    config::Config,
    model::{Provider, Snapshot},
    providers,
    store::Store,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Insights,
    Activity,
    Models,
    Limits,
    Connections,
    Settings,
}
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Accounts",
            Self::Insights => "Insights",
            Self::Activity => "Activity",
            Self::Models => "Models",
            Self::Limits => "Limits",
            Self::Connections => "Connections",
            Self::Settings => "Settings",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Insights => "insights",
            Self::Activity => "activity",
            Self::Models => "models",
            Self::Limits => "limits",
            Self::Connections => "connections",
            Self::Settings => "settings",
        }
    }
    /// Pages whose numbers follow the selected period.
    fn uses_period(self) -> bool {
        matches!(
            self,
            Self::Overview | Self::Insights | Self::Activity | Self::Models
        )
    }
}
#[derive(Clone)]
enum UiAction {
    Noop,
    Page(Page),
    Period(Period),
    Provider(Option<Provider>),
    AccountActivity(Provider),
    Sort(Table, Column),
    ResetSort(Table),
    CostFilter(Table, CostFilter),
    ClearFilters(Table),
    Refresh,
    Export,
    Privacy,
    Online,
    Dark,
    Compact,
    Quit,
    Metric(bool),
    AddHome(Provider),
    RemoveHome(Provider, usize),
    ImportCursor,
    ClearCursor,
    Interval(u64),
    OpenProvider(Provider),
    ActivityPage(usize),
    ActivityRows(Option<usize>),
    Detail(String),
    CloseDetail,
    CheckUpdate,
    UpdateNow,
    UpdateDelay,
}

enum RefreshMsg {
    /// The cached history, read from disk after the window opens.
    Loaded(Option<Snapshot>),
    Update(Arc<Snapshot>),
    Done(Result<Arc<Snapshot>, String>),
    Message(String),
    UpdateChecked {
        manual: bool,
        result: Result<token_dock::update::Check, String>,
    },
    UpdateInstalled(Result<token_dock::update::Install, String>),
}

pub struct Dashboard {
    config: Config,
    /// Shared with background work, so handing it off never copies the history.
    live: Arc<Snapshot>,
    /// Bumped whenever derived values must be recomputed.
    version: u64,
    /// The instant derived values are computed for; advances with `version`.
    now: chrono::DateTime<Utc>,
    /// Bumped on every settings change.
    config_revision: u64,
    derived: derived::Derived,
    has_records: [bool; Provider::COUNT],
    loading: bool,
    saver: Sender<Config>,
    page: Page,
    period: Period,
    provider: Option<Provider>,
    token_chart: bool,
    /// Hovered chart and bar index, for dimming the other bars.
    hovered: Option<(&'static str, usize)>,
    refreshing: bool,
    pending_refresh: bool,
    tray_attempted: bool,
    tray_error: Option<String>,
    last_minute: Instant,
    receiver: Receiver<RefreshMsg>,
    sender: Sender<RefreshMsg>,
    last_refresh: Instant,
    focus: FocusHandle,
    toast: Option<(String, Instant)>,
    tray: Option<tray::Tray>,
    compact: bool,
    pending_size: Option<Size<Pixels>>,
    activity_page: usize,
    model_query: Entity<InputState>,
    activity_query: Entity<InputState>,
    model_sort: Sort,
    activity_sort: Sort,
    project_sort: Sort,
    model_cost: CostFilter,
    activity_cost: CostFilter,
    _input_subscriptions: Vec<Subscription>,
    /// `None` shows as many rows as fit in the window.
    activity_rows: Option<usize>,
    detail: Option<String>,
    update_offer: Option<token_dock::update::Offer>,
    /// The launch prompt is shown at most once. Delay does not clear this.
    update_asked: bool,
    update_auto: bool,
    update_checking: bool,
    update_busy: bool,
}

impl Dashboard {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (config, error) = match Config::load() {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(error.to_string())),
        };
        // History can be large: read it after the window is up.
        {
            let sender = sender.clone();
            std::thread::spawn(move || {
                let snapshot = Store::open().and_then(|store| store.load()).ok().flatten();
                let _ = sender.send(RefreshMsg::Loaded(snapshot));
            });
        }
        // Settings are written by one thread in order, keeping only the latest.
        let (saver, saves) = mpsc::channel::<Config>();
        {
            let sender = sender.clone();
            std::thread::spawn(move || {
                while let Ok(mut config) = saves.recv() {
                    while let Ok(newer) = saves.try_recv() {
                        config = newer;
                    }
                    if let Err(error) = config.save() {
                        let _ = sender.send(RefreshMsg::Message(error.to_string()));
                    }
                }
            });
        }
        token_dock::pricing::start(config.online);
        sync_components(config.dark, window, cx);
        let focus = cx.focus_handle();
        window.focus(&focus);
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            if weak
                .update(cx, |view, _| view.tray.is_some())
                .unwrap_or(false)
            {
                window::hide(window, cx);
                false
            } else {
                cx.quit();
                true
            }
        });
        let model_query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search models or accounts")
                .clean_on_escape()
        });
        let activity_query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search models, accounts or projects")
                .clean_on_escape()
        });
        let input_subscriptions = vec![
            // The input paints the keystroke itself. Filtering starts in the
            // background at once and replaces any search still running.
            cx.subscribe(&model_query, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.ensure_derived(cx);
                }
            }),
            cx.subscribe(&activity_query, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.activity_page = 0;
                    view.ensure_derived(cx);
                    cx.notify();
                }
            }),
        ];
        let view = Self {
            config,
            live: Arc::new(Snapshot::default()),
            version: 0,
            now: Utc::now(),
            config_revision: 0,
            derived: Default::default(),
            has_records: [false; Provider::COUNT],
            loading: true,
            saver,
            page: Page::Overview,
            period: Period::Month,
            provider: None,
            token_chart: true,
            hovered: None,
            refreshing: false,
            pending_refresh: false,
            tray_attempted: false,
            tray_error: None,
            last_minute: Instant::now(),
            receiver,
            sender,
            last_refresh: Instant::now(),
            focus,
            toast: error.map(|error| (error, Instant::now())),
            tray: None,
            compact: false,
            pending_size: None,
            activity_page: 0,
            model_query,
            activity_query,
            model_sort: Sort::new(Column::Tokens),
            activity_sort: Sort::new(Column::Time),
            project_sort: Sort::new(Column::Tokens),
            model_cost: CostFilter::All,
            activity_cost: CostFilter::All,
            _input_subscriptions: input_subscriptions,
            activity_rows: None,
            detail: None,
            update_offer: None,
            update_asked: false,
            update_auto: false,
            update_checking: false,
            update_busy: false,
        };
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if cx
                    .update(|window, cx| view.update(cx, |view, cx| view.tick(window, cx)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view
    }
    fn data(&self) -> &Snapshot {
        &self.live
    }
    fn theme(&self) -> Theme {
        Theme::new(self.config.dark)
    }
    fn money(&self, value: f64) -> String {
        if self.config.privacy {
            analytics::hidden_value().into()
        } else {
            format!("${value:.2}")
        }
    }
    fn number(&self, value: u64) -> String {
        if self.config.privacy {
            analytics::hidden_value().into()
        } else {
            analytics::compact(value)
        }
    }
    fn message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.toast = Some((text.into(), Instant::now()));
        cx.notify();
    }
    /// Queues the settings for the saver thread; the UI never waits on disk.
    fn save(&mut self, cx: &mut Context<Self>) {
        self.config_revision += 1;
        let _ = self.saver.send(self.config.clone());
        cx.notify();
    }
    /// Marks every derived value out of date.
    fn invalidate(&mut self) {
        self.version += 1;
        self.now = Utc::now();
    }
    fn set_live(&mut self, snapshot: Arc<Snapshot>) {
        let mut has_records = [false; Provider::COUNT];
        for record in &snapshot.records {
            has_records[record.provider.index()] = true;
        }
        self.has_records = has_records;
        self.live = snapshot;
        self.invalidate();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing || self.loading {
            self.pending_refresh = true;
            return;
        }
        self.refreshing = true;
        self.last_refresh = Instant::now();
        let config = self.config.clone();
        let previous = self.live.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let sender_done = sender.clone();
            // Accounts report in bursts. Publish at most a few intermediate
            // copies of the history; the final result always follows.
            let last = std::cell::Cell::new(None::<Instant>);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                providers::collect_with(&config, &previous, |snapshot| {
                    if last
                        .get()
                        .is_none_or(|at| at.elapsed() >= Duration::from_millis(300))
                    {
                        last.set(Some(Instant::now()));
                        let _ = sender.send(RefreshMsg::Update(Arc::new(snapshot.clone())));
                    }
                })
            }));
            let result = match result {
                Ok(result) => result.map(Arc::new).map_err(|error| error.to_string()),
                Err(_) => Err("Usage scan stopped unexpectedly. Refresh to retry.".into()),
            };
            let _ = sender_done.send(RefreshMsg::Done(result));
        });
        cx.notify();
    }
    fn check_update(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.update_checking || self.update_busy {
            if manual {
                self.message("Checking for updates.", cx);
            }
            return;
        }
        self.update_checking = true;
        if manual {
            self.message("Checking for updates.", cx);
        }
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = token_dock::update::check();
            let _ = sender.send(RefreshMsg::UpdateChecked { manual, result });
        });
        cx.notify();
    }
    fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self.update_offer.clone() else {
            return;
        };
        if self.update_busy {
            return;
        }
        self.update_busy = true;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = token_dock::update::install(&offer);
            let _ = sender.send(RefreshMsg::UpdateInstalled(result));
        });
        cx.notify();
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(size) = self.pending_size.take() {
            window.resize(size);
            cx.notify();
        }
        if !self.tray_attempted {
            self.tray_attempted = true;
            match tray::create() {
                Ok(tray) => {
                    self.tray = Some(tray);
                    window::configure_tray_app();
                    #[cfg(target_os = "macos")]
                    window::show(window, cx);
                    self.update_tray();
                }
                Err(error) => {
                    self.tray_error = Some(error.to_string());
                    self.message(
                        "System tray could not be initialized. Close the window to quit.",
                        cx,
                    );
                }
            }
        }
        if !self.update_auto {
            self.update_auto = true;
            self.check_update(false, cx);
        }
        // Apply only the newest history among this tick's messages.
        let mut latest = None;
        let mut restart_for_update = false;
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                RefreshMsg::Loaded(snapshot) => {
                    self.loading = false;
                    if let Some(mut snapshot) = snapshot {
                        for provider in &mut snapshot.providers {
                            provider.stale = true;
                        }
                        latest = Some(Arc::new(snapshot));
                    }
                    self.pending_refresh = true;
                }
                RefreshMsg::Update(snapshot) => latest = Some(snapshot),
                RefreshMsg::Done(result) => {
                    self.refreshing = false;
                    match result {
                        Ok(snapshot) => latest = Some(snapshot),
                        Err(error) => self.message(error, cx),
                    }
                }
                RefreshMsg::Message(text) => self.message(text, cx),
                RefreshMsg::UpdateChecked { manual, result } => {
                    self.update_checking = false;
                    match result {
                        Ok(token_dock::update::Check::Available(offer)) => {
                            let show = manual
                                || token_dock::update::prompt_on_launch(
                                    &offer.version,
                                    token_dock::update::current_version(),
                                    self.update_asked,
                                );
                            if show {
                                self.update_asked = true;
                                self.update_offer = Some(offer);
                            }
                        }
                        Ok(token_dock::update::Check::Current) => {
                            self.update_asked = true;
                            self.update_offer = None;
                            if manual {
                                self.message("TokenDock is up to date.", cx);
                            }
                        }
                        Ok(token_dock::update::Check::MissingDownload { version }) => {
                            if manual {
                                self.message(
                                    format!("Version {version} has no download for this system."),
                                    cx,
                                );
                            }
                        }
                        Ok(token_dock::update::Check::Unsupported) => {
                            if manual {
                                self.message(
                                    "Updates are available for Windows x64 and Apple Silicon Macs.",
                                    cx,
                                );
                            }
                        }
                        Err(error) => {
                            if manual {
                                self.message(error, cx);
                            } else {
                                self.update_asked = true;
                            }
                        }
                    }
                }
                RefreshMsg::UpdateInstalled(result) => {
                    self.update_busy = false;
                    match result {
                        Ok(token_dock::update::Install::Restarting) => {
                            self.message("Restarting to finish the update.", cx);
                            restart_for_update = true;
                        }
                        Ok(token_dock::update::Install::Opened(text)) => {
                            self.update_offer = None;
                            self.message(text, cx);
                        }
                        Err(text) => self.message(text, cx),
                    }
                }
            }
            cx.notify();
        }
        if restart_for_update {
            cx.quit();
            return;
        }
        if let Some(snapshot) = latest {
            self.set_live(snapshot);
        }
        if self.pending_refresh && !self.refreshing && !self.loading {
            self.pending_refresh = false;
            self.refresh(cx);
        }
        if token_dock::pricing::take_updated() {
            self.invalidate();
            cx.notify();
        }
        if self.last_minute.elapsed() >= Duration::from_secs(60) {
            self.last_minute = Instant::now();
            self.invalidate();
            cx.notify();
        }
        if self.tray.is_some() {
            self.ensure_today(cx);
        }
        if !self.refreshing
            && !self.loading
            && self.last_refresh.elapsed() >= Duration::from_secs(self.config.refresh_seconds)
        {
            self.refresh(cx);
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, time)| time.elapsed() > Duration::from_secs(12))
        {
            self.toast = None;
            cx.notify();
        }
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if let Some(tray) = &self.tray {
                let action = if event.id == tray.overview {
                    Some(UiAction::Page(Page::Overview))
                } else if event.id == tray.compact {
                    Some(UiAction::Compact)
                } else if event.id == tray.refresh {
                    Some(UiAction::Refresh)
                } else if event.id == tray.quit {
                    Some(UiAction::Quit)
                } else {
                    None
                };
                if let Some(action) = action {
                    if matches!(action, UiAction::Compact) {
                        self.compact = false;
                    }
                    let show_window = matches!(action, UiAction::Page(_) | UiAction::Compact);
                    let quit = matches!(action, UiAction::Quit);
                    self.perform(action, window, cx);
                    if quit {
                        return;
                    }
                    if show_window {
                        window::show(window, cx);
                    }
                }
            }
        }
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if !cfg!(target_os = "macos")
                && matches!(
                    event,
                    tray_icon::TrayIconEvent::Click {
                        button: tray_icon::MouseButton::Left,
                        button_state: tray_icon::MouseButtonState::Up,
                        ..
                    }
                )
            {
                self.compact = true;
                self.resize_window(window, size(px(440.), px(700.)));
                window.focus(&self.focus);
                window::show(window, cx);
                cx.notify();
            }
        }
    }
    fn visible_accounts(&self) -> Vec<&token_dock::model::ProviderStatus> {
        self.data()
            .providers
            .iter()
            .filter(|status| {
                self.config.enabled[status.provider.index()]
                    && (status.detected
                        || status.connected
                        || self.has_records[status.provider.index()])
            })
            .collect()
    }
    fn resize_window(&mut self, window: &mut Window, size: Size<Pixels>) {
        self.pending_size = None;
        if window::restore_before_resize(window) {
            // Windows restores asynchronously. Resize after its placement has settled.
            self.pending_size = Some(size);
        } else {
            window.resize(size);
        }
    }
    fn spend(&self, totals: &analytics::Totals) -> String {
        totals
            .spend()
            .map(|cost| self.money(cost))
            .unwrap_or_else(|| analytics::empty_value().into())
    }
    fn cost_caption(&self, totals: &analytics::Totals) -> String {
        if totals.events == 0 {
            return "No usage".into();
        }
        if totals.reported_rows + totals.estimated_rows > 0 {
            let sep = analytics::separator();
            let caption = match (totals.reported_rows, totals.estimated_rows) {
                (_, 0) => "Reported".to_string(),
                (0, _) => format!("Estimated{sep}API rates"),
                _ => "Reported + estimated".to_string(),
            };
            return if totals.events > totals.reported_rows + totals.estimated_rows {
                match (totals.reported_rows, totals.estimated_rows) {
                    (_, 0) => format!("Partial{sep}reported"),
                    (0, _) => format!("Partial{sep}estimated"),
                    _ => format!("Partial{sep}reported + estimated"),
                }
            } else {
                caption
            };
        }
        match token_dock::pricing::state() {
            token_dock::pricing::RateState::Loading => "Loading model rates".into(),
            token_dock::pricing::RateState::Unavailable => "Model rates unavailable".into(),
            token_dock::pricing::RateState::Ready => "No public rate".into(),
        }
    }
    fn event_cost(&self, record: &token_dock::model::UsageRecord) -> (String, String) {
        match analytics::record_cost(record) {
            Some((cost, false)) => (self.money(cost), "Reported".into()),
            Some((cost, true)) => (
                self.money(cost),
                format!("Estimated{}API rate", analytics::separator()),
            ),
            None => (analytics::empty_value().into(), "No public rate".into()),
        }
    }
    /// Uses the cached figures for today; runs again whenever they are recomputed.
    fn update_tray(&mut self) {
        if let Some(tray) = &self.tray {
            let Some(analysis) = self.derived.today.value() else {
                return;
            };
            let mut tooltip = format!(
                "TokenDock\n{} tokens today",
                self.number(analysis.totals.tokens.total())
            );
            for status in self.visible_accounts() {
                if let Some(limit) = status.limits.first() {
                    let remaining = if self.config.privacy {
                        "Hidden".into()
                    } else if limit.expired(Utc::now()) {
                        "Refresh needed".into()
                    } else {
                        format!(
                            "{:.0}% left{}",
                            limit.remaining(),
                            if status.stale { " (cached)" } else { "" }
                        )
                    };
                    tooltip.push_str(&format!("\n{}: {remaining}", status.provider.name()));
                }
            }
            let _ = tray
                .icon
                .set_tooltip(Some(tooltip.chars().take(120).collect::<String>()));
            #[cfg(target_os = "macos")]
            {
                let title = if self.config.privacy {
                    "TokenDock".into()
                } else {
                    self.visible_accounts()
                        .iter()
                        .filter_map(|status| {
                            status
                                .limits
                                .first()
                                .filter(|limit| !limit.expired(Utc::now()))
                                .map(|limit| {
                                    format!(
                                        "{} {:.0}%{}",
                                        status.provider.name(),
                                        limit.remaining(),
                                        if status.stale { "~" } else { "" }
                                    )
                                })
                        })
                        .collect::<Vec<_>>()
                        .join(analytics::separator())
                };
                tray.icon.set_title(Some(if title.is_empty() {
                    "TokenDock".into()
                } else {
                    title
                }));
            }
        }
    }
    fn perform(&mut self, action: UiAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            UiAction::Noop => {}
            UiAction::Page(page) => {
                self.page = page;
                if page == Page::Overview {
                    self.provider = None;
                }
                self.detail = None;
                if self.compact {
                    self.compact = false;
                    self.resize_window(window, size(px(1280.), px(860.)));
                }
                window.focus(&self.focus);
            }
            UiAction::Period(period) => {
                self.period = period;
                self.activity_page = 0;
                self.hovered = None;
            }
            UiAction::Provider(provider) => {
                self.provider = provider;
                self.activity_page = 0;
                self.hovered = None;
            }
            UiAction::Refresh => self.refresh(cx),
            UiAction::AccountActivity(provider) => {
                self.provider = Some(provider);
                self.page = Page::Activity;
                self.activity_page = 0;
                self.activity_cost = CostFilter::All;
                self.activity_query
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.detail = None;
                window.focus(&self.focus);
            }
            UiAction::Sort(table, column) => match table {
                Table::Models => self.model_sort.select(column),
                Table::Activity => {
                    self.activity_sort.select(column);
                    self.activity_page = 0;
                }
                Table::Projects => self.project_sort.select(column),
            },
            UiAction::ResetSort(table) => match table {
                Table::Models => self.model_sort.reset(),
                Table::Activity => {
                    self.activity_sort.reset();
                    self.activity_page = 0;
                }
                Table::Projects => self.project_sort.reset(),
            },
            UiAction::CostFilter(table, filter) => match table {
                Table::Models => self.model_cost = filter,
                Table::Activity | Table::Projects => {
                    self.activity_cost = filter;
                    self.activity_page = 0;
                }
            },
            UiAction::ClearFilters(table) => {
                self.provider = None;
                match table {
                    Table::Models => {
                        self.model_cost = CostFilter::All;
                        self.model_query
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    Table::Activity | Table::Projects => {
                        self.activity_cost = CostFilter::All;
                        self.activity_page = 0;
                        self.activity_query
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                }
            }
            UiAction::Export => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name("usage.csv")
                    .save_file()
                {
                    // Writing can take a while for long histories; do it off the UI thread.
                    let activity = (self.page == Page::Activity)
                        .then(|| self.derived.activity.value())
                        .flatten();
                    let (data, config) = (self.live.clone(), self.config.clone());
                    let (period, provider) = (self.period, self.provider);
                    self.message("Exporting...", cx);
                    cx.spawn(async move |view, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                let filtered = activity.map(|activity| Snapshot {
                                    records: activity.records().cloned().collect(),
                                    ..Default::default()
                                });
                                analytics::export_csv(
                                    &path,
                                    filtered.as_ref().unwrap_or(&data),
                                    &config,
                                    period,
                                    provider,
                                )
                            })
                            .await;
                        let _ = view.update(cx, |view, cx| match result {
                            Ok(count) => {
                                view.message(format!("Exported {count} usage records"), cx)
                            }
                            Err(error) => view.message(error.to_string(), cx),
                        });
                    })
                    .detach();
                }
            }
            UiAction::Online => {
                self.config.online = !self.config.online;
                self.save(cx);
                self.refresh(cx);
                if !self.config.online {
                    self.message(
                        "Network disabled for future refreshes. Current requests may finish.",
                        cx,
                    );
                }
            }
            UiAction::Privacy => {
                self.config.privacy = !self.config.privacy;
                if self.config.privacy {
                    self.activity_query
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.save(cx);
                self.update_tray();
            }
            UiAction::Dark => {
                self.config.dark = !self.config.dark;
                sync_components(self.config.dark, window, cx);
                self.save(cx);
            }
            UiAction::Compact => {
                self.compact = !self.compact;
                let size = if self.compact {
                    size(px(440.), px(700.))
                } else {
                    size(px(1280.), px(860.))
                };
                self.resize_window(window, size);
                window.focus(&self.focus);
            }
            UiAction::Quit => cx.quit(),
            UiAction::CheckUpdate => self.check_update(true, cx),
            UiAction::UpdateNow => self.install_update(cx),
            UiAction::UpdateDelay => {
                if let Some(offer) = self.update_offer.take() {
                    self.config.update_delayed = Some(offer.version);
                    self.save(cx);
                    self.message("Update delayed. The next launch asks again.", cx);
                }
            }
            UiAction::Metric(tokens) => {
                self.token_chart = tokens;
                self.hovered = None;
            }
            UiAction::AddHome(provider) => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(format!("Choose {} data folder", provider.name()))
                    .pick_folder()
                {
                    let paths = &mut self.config.custom_homes[provider.index()];
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                    self.save(cx);
                    self.refresh(cx);
                }
            }
            UiAction::RemoveHome(provider, index) => {
                let paths = &mut self.config.custom_homes[provider.index()];
                if index < paths.len() {
                    paths.remove(index);
                    self.save(cx);
                    self.refresh(cx);
                }
            }
            UiAction::ImportCursor => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Cursor usage export", &["csv"])
                    .pick_file()
                {
                    self.message("Reading Cursor export...", cx);
                    cx.spawn(async move |view, cx| {
                        let read = path.clone();
                        let result = cx
                            .background_executor()
                            .spawn(async move { providers::history::read_cursor_csv(&read) })
                            .await;
                        let _ = view.update(cx, |view, cx| match result {
                            Ok(records) => {
                                view.config.cursor_imports = vec![path];
                                view.save(cx);
                                view.refresh(cx);
                                view.message(
                                    format!("Imported {} Cursor events", records.len()),
                                    cx,
                                );
                            }
                            Err(error) => view.message(error.to_string(), cx),
                        });
                    })
                    .detach();
                }
            }
            UiAction::ClearCursor => {
                self.config.cursor_imports.clear();
                self.save(cx);
                self.refresh(cx);
            }
            UiAction::Interval(seconds) => {
                self.config.refresh_seconds = seconds;
                self.save(cx);
            }
            UiAction::OpenProvider(provider) => {
                let url = match provider {
                    Provider::Codex => "https://chatgpt.com/codex/settings/usage",
                    Provider::Claude => "https://claude.ai/settings/usage",
                    Provider::Cursor => "https://cursor.com/dashboard?tab=usage",
                    Provider::Kiro => "https://app.kiro.dev/",
                    Provider::Antigravity => "https://antigravity.google/",
                    Provider::OpenCode => "https://opencode.ai/zen",
                };
                if open::that(url).is_err() {
                    self.message("The provider dashboard could not be opened", cx);
                }
            }
            UiAction::ActivityPage(page) => self.activity_page = page,
            UiAction::ActivityRows(limit) => {
                self.activity_rows = limit;
                self.activity_page = 0;
            }
            UiAction::Detail(id) => self.detail = Some(id),
            UiAction::CloseDetail => self.detail = None,
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let typing = self
            .model_query
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            || self
                .activity_query
                .read(cx)
                .focus_handle(cx)
                .is_focused(window);
        if typing && !mods.control && !mods.platform {
            return;
        }
        let action = if (mods.control || mods.platform) && mods.shift {
            match key {
                "1" => Some(UiAction::Period(Period::TwoHours)),
                "2" => Some(UiAction::Period(Period::Day)),
                "3" => Some(UiAction::Period(Period::Week)),
                "4" => Some(UiAction::Period(Period::Month)),
                "5" => Some(UiAction::Period(Period::Quarter)),
                _ => None,
            }
        } else if mods.control || mods.platform {
            match key {
                "r" => Some(UiAction::Refresh),
                "p" => Some(UiAction::Privacy),
                "q" => Some(UiAction::Quit),
                "1" => Some(UiAction::Page(Page::Overview)),
                "2" => Some(UiAction::Page(Page::Insights)),
                "3" => Some(UiAction::Page(Page::Models)),
                "4" => Some(UiAction::Page(Page::Activity)),
                "5" => Some(UiAction::Page(Page::Limits)),
                "6" => Some(UiAction::Page(Page::Connections)),
                "7" => Some(UiAction::Page(Page::Settings)),
                _ => None,
            }
        } else {
            match key {
                "c" => Some(UiAction::Metric(false)),
                "t" => Some(UiAction::Metric(true)),
                "l" => Some(UiAction::Page(Page::Limits)),
                "escape" => Some(UiAction::CloseDetail),
                "tab" => {
                    if mods.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    None
                }
                _ => None,
            }
        };
        if let Some(action) = action {
            self.perform(action, window, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Starts background work for anything out of date; renders what is ready.
        self.ensure_derived(cx);
        let theme = self.theme();
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        let rail = width < 1000.;
        let content = if self.compact {
            self.compact_view(cx).into_any_element()
        } else {
            let page = match self.page {
                Page::Overview => self.overview(width, height, cx),
                Page::Insights => self.insights_view(width, height, cx),
                Page::Activity => self.activity(width, height, cx),
                Page::Models => self.models_view(width, height, cx),
                Page::Limits => self.limits_view(width, height, cx),
                Page::Connections => self.connections(cx),
                Page::Settings => self.settings(width, cx),
            };
            div()
                .flex()
                .size_full()
                .child(self.sidebar(rail, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(self.header(width, height, cx))
                        .child({
                            let pinned =
                                matches!(self.page, Page::Overview | Page::Activity | Page::Models);
                            div()
                                .id(SharedString::from(format!(
                                    "main-scroll-{}",
                                    self.page.title()
                                )))
                                .flex_1()
                                .min_h_0()
                                .p(px(layout::page_padding(width, height)))
                                .when(pinned, |region| region.flex().flex_col().overflow_hidden())
                                .when(!pinned, |region| region.overflow_y_scroll())
                                .child(page)
                        }),
                )
                .into_any_element()
        };
        div()
            .id("root")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.ink))
            .font_family(body_font())
            .text_size(px(TYPE_BODY))
            .line_height(relative(1.4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    if !view.focus.contains_focused(window, cx) {
                        window.focus(&view.focus);
                    }
                }),
            )
            .on_key_down(cx.listener(Self::key))
            .child(content)
            .when(self.update_offer.is_some(), |root| {
                root.child(self.update_banner(cx))
            })
            .when_some(self.toast.as_ref(), |root, (message, _)| {
                root.child(
                    div()
                        .absolute()
                        .bottom(px(SPACE_L))
                        .right(px(SPACE_L))
                        .max_w(px(480.))
                        .rounded(px(PANEL_RADIUS))
                        .bg(rgb(theme.ink))
                        .text_color(rgb(theme.panel))
                        .text_size(px(TYPE_BODY))
                        .px(px(SPACE_M))
                        .py(px(SPACE_S))
                        .shadow_lg()
                        .child(message.clone()),
                )
            })
    }
}
pub fn assets() -> Assets {
    Assets
}
