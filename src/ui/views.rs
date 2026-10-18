use super::*;
use chrono::Local;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use token_usage::{
    analytics::Totals,
    model::{LimitWindow, Provider, ProviderStatus, UsageRecord},
};

/// Every table row has the same height so tables line up across pages.
pub(super) const ROW_HEIGHT: f32 = 40.;
pub(super) const TABLE_HEADER: f32 = 36.;
const TABLE_FOOTER: f32 = 44.;
const FILTER_GAP: f32 = SPACE_S;
/// Inset and gap shared by a table header and every row under it.
pub(super) const TABLE_PAD: f32 = SPACE_M;
pub(super) const COLUMN_GAP: f32 = SPACE_S;
/// Panel border on each side. Header and body both sit inside it.
const TABLE_BORDER: f32 = 1.;

fn filters_height(content_width: f32) -> f32 {
    if content_width < 650. {
        CONTROL_HEIGHT * 2. + SPACE_XS
    } else {
        CONTROL_HEIGHT
    }
}

/// Horizontal inset of a nav row. The rail centers its icon; the wide sidebar
/// uses one space step so the icon lines up with the brand mark.
fn nav_inset(rail: bool) -> f32 {
    if rail {
        (layout::sidebar_width(0.) - SPACE_XS * 2. - ICON_BODY) / 2.
    } else {
        SPACE_XS
    }
}

impl Dashboard {
    // ----- Chrome -------------------------------------------------------------

    pub(super) fn sidebar(&self, rail: bool, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let pad = if rail { SPACE_XS } else { SPACE_S };
        let mark = 22.;
        // Rail centers the mark in the column. Expanded, it shares the nav icon inset.
        let brand_px = if rail {
            (layout::sidebar_width(0.) - mark) / 2.
        } else {
            pad + nav_inset(false)
        };
        let label_inset = pad + nav_inset(false) + ICON_BODY + SPACE_XS;
        div()
            .w(px(layout::sidebar_width(if rail { 0. } else { 1000. })))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(theme.sidebar))
            .border_r_1()
            .border_color(rgb(theme.line))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(layout::HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px(px(brand_px))
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(
                        div()
                            .size(px(mark))
                            .flex_shrink_0()
                            .rounded(px(RADIUS))
                            .bg(rgb(0x252a25))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .relative()
                                    .size(px(mark * 0.76))
                                    .child(
                                        gpui::svg()
                                            .path("brand")
                                            .absolute()
                                            .size_full()
                                            .text_color(rgb(0xf4f2e8)),
                                    )
                                    .child(
                                        gpui::svg()
                                            .path("brand-needle")
                                            .absolute()
                                            .size_full()
                                            .text_color(rgb(0xa2c784)),
                                    ),
                            ),
                    )
                    .when(!rail, |row| {
                        row.child(title("Token Usage", theme).min_w_0().truncate())
                    }),
            )
            .child(
                div()
                    .px(px(pad))
                    .pt(px(SPACE_XS))
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_2XS))
                    .children(
                        [
                            Page::Overview,
                            Page::Insights,
                            Page::Models,
                            Page::Activity,
                            Page::Limits,
                        ]
                        .into_iter()
                        .map(|page| self.navigation(page, rail, cx)),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px(px(pad))
                    .pb(px(SPACE_S))
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_2XS))
                    .child(self.navigation(Page::Connections, rail, cx))
                    .child(self.navigation(Page::Settings, rail, cx)),
            )
            .when(!rail, |sidebar| {
                sidebar.child(
                    caption(concat!("v", env!("CARGO_PKG_VERSION")), theme)
                        .px(px(label_inset))
                        .pb(px(SPACE_M)),
                )
            })
    }
    fn navigation(&self, page: Page, rail: bool, cx: &Context<Self>) -> Stateful<Div> {
        let theme = self.theme();
        let active = self.page == page;
        div()
            .id(SharedString::from(format!("nav-{}", page.title())))
            .focusable()
            .tab_index(0)
            .h(px(CONTROL_HEIGHT))
            .rounded(px(RADIUS))
            .border_1()
            .border_color(gpui::transparent_black())
            .px(px(nav_inset(rail)))
            .flex()
            .items_center()
            .gap(px(SPACE_XS))
            .cursor_pointer()
            .text_size(px(TYPE_BODY))
            .when(active, |item| {
                item.bg(rgb(theme.wash))
                    .text_color(rgb(theme.ink))
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!active, |item| {
                item.text_color(rgb(theme.muted))
                    .hover(move |style| style.bg(rgb(theme.wash)).text_color(rgb(theme.ink)))
            })
            .focus(move |style| style.border_color(rgb(theme.focus)))
            .child(
                icon(page.icon(), theme)
                    .size(px(ICON_BODY))
                    .text_color(rgb(if active { theme.ink } else { theme.muted })),
            )
            .when(!rail, |item| item.child(page.title()))
            .when(rail, |item| {
                item.tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(page.title()).build(window, cx)
                })
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.perform(UiAction::Page(page), window, cx)
            }))
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    view.perform(UiAction::Page(page), window, cx);
                    cx.stop_propagation();
                }
            }))
    }
    pub(super) fn header(&self, width: f32, height: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let content = width - layout::sidebar_width(width);
        let updated = if self.refreshing {
            "Refreshing...".to_string()
        } else if self.loading {
            "Loading history...".to_string()
        } else {
            self.data()
                .refreshed_at
                .map(|time| format!("Updated {}", time.with_timezone(&Local).format("%H:%M")))
                .unwrap_or_else(|| "Not synced".into())
        };
        div()
            .h(px(layout::HEADER_HEIGHT))
            .flex_shrink_0()
            .px(px(layout::page_padding(width, height)))
            .border_b_1()
            .border_color(rgb(theme.line))
            .flex()
            .items_center()
            .gap(px(SPACE_XS))
            .child(
                div()
                    .min_w_0()
                    .text_size(px(TYPE_DISPLAY))
                    .line_height(relative(1.1))
                    .font_weight(FontWeight::SEMIBOLD)
                    .font_family(display_font())
                    .truncate()
                    .child(self.page.title()),
            )
            .child(div().flex_1())
            .when(content >= 760., |row| row.child(caption(updated, theme)))
            .when(self.page.uses_period(), |row| {
                row.child(self.period_control(content < 560., cx))
            })
            .child(self.icon_button(
                "header-refresh",
                "refresh",
                "Refresh",
                UiAction::Refresh,
                cx,
            ))
            .child(self.icon_button(
                "header-theme",
                if self.config.dark {
                    "theme-light"
                } else {
                    "theme"
                },
                if self.config.dark {
                    "Switch to light"
                } else {
                    "Switch to dark"
                },
                UiAction::Dark,
                cx,
            ))
            .when(content >= 560., |row| {
                row.child(self.icon_button(
                    "header-compact",
                    "overview",
                    "Quick view",
                    UiAction::Compact,
                    cx,
                ))
            })
    }

    // ----- Shared pieces --------------------------------------------------

    fn page_body(&self) -> Div {
        div()
            .w_full()
            .max_w(px(1200.))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(SPACE_S))
    }
    /// Centered empty copy. Callers add an action with `.child` when one exists.
    fn empty_block(&self, heading: &str, detail: Option<&str>) -> Div {
        let theme = self.theme();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(SPACE_XS))
            .child(title(heading.to_owned(), theme))
            .when_some(detail, |column, detail| {
                column.child(caption(detail.to_owned(), theme))
            })
    }
    pub(super) fn stat_tile(
        &self,
        name: &str,
        value: String,
        note: Option<String>,
        dense: bool,
    ) -> Div {
        let theme = self.theme();
        panel(theme)
            .flex_1()
            .min_w_0()
            .p(px(if dense { SPACE_S } else { SPACE_M }))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(SPACE_2XS))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(label(name.to_owned(), theme))
                    .child(div().flex_1())
                    .when_some(note, |row, note| {
                        row.child(caption(note, theme).min_w_0().truncate())
                    }),
            )
            .child(
                tabular(div())
                    .font_family(display_font())
                    .text_size(px(if dense { TYPE_DISPLAY } else { TYPE_FIGURE }))
                    .line_height(relative(1.15))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(value),
            )
    }
    /// An unboxed figure for the home summary band. The lead figure is set
    /// larger; the rest sit behind a hairline so the band reads as one ledger.
    fn summary_figure(
        &self,
        name: &str,
        value: String,
        note: Option<String>,
        lead: bool,
        dense: bool,
    ) -> Div {
        let theme = self.theme();
        let size = match (lead, dense) {
            (true, false) => TYPE_LEAD,
            (true, true) => TYPE_FIGURE,
            (false, false) => TYPE_DISPLAY,
            (false, true) => TYPE_TITLE,
        };
        tabular(
            div()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .gap(px(SPACE_2XS))
                .when(lead, |figure| {
                    figure.w(relative(0.36)).flex_shrink_0().pr(px(SPACE_M))
                })
                .when(!lead, |figure| {
                    figure
                        .flex_1()
                        .pl(px(if dense { SPACE_S } else { SPACE_M }))
                        .border_l_1()
                        .border_color(rgb(theme.line))
                })
                .child(label(name.to_owned(), theme))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(SPACE_XS))
                        .child(
                            div()
                                .font_family(display_font())
                                .text_size(px(size))
                                .line_height(relative(1.05))
                                .font_weight(if lead {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .text_color(rgb(theme.ink))
                                .flex_shrink_0()
                                .child(value),
                        )
                        .when_some(note, |row, note| {
                            row.child(caption(note, theme).min_w_0().truncate())
                        }),
                ),
        )
    }
    /// Inline status text. A live reading earns a small dot; nothing is boxed.
    fn badge(&self, text: &str, active: bool) -> Div {
        let theme = self.theme();
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(SPACE_2XS))
            .text_size(px(TYPE_CAPTION))
            .text_color(rgb(if active { theme.ink } else { theme.muted }))
            .when(active, |row| {
                row.child(
                    div()
                        .size(px(STATUS_PIP))
                        .flex_shrink_0()
                        .rounded(px(STATUS_PIP / 2.))
                        .bg(rgb(theme.focus)),
                )
            })
            .child(text.to_owned())
    }
    pub(super) fn table_cell(
        &self,
        text: &str,
        width: Option<f32>,
        numeric: bool,
        muted: bool,
    ) -> Div {
        let theme = self.theme();
        // Numeric headers pack their label to the column's right edge. The
        // value has to do the same: text_align on a shrink-wrapped text node
        // stays on the left, so the number never sits under the heading.
        // Tabular figures stay on the numbers, not on the names beside them.
        let cell = div()
            .when_some(width, |cell, width| {
                cell.w(px(width))
                    .min_w(px(0.))
                    .flex_shrink_0()
                    .overflow_hidden()
            })
            .flex()
            .items_center()
            .min_w(px(0.))
            .when(numeric, |cell| cell.justify_end())
            .text_size(px(TYPE_BODY))
            .truncate()
            .text_color(rgb(if muted { theme.muted } else { theme.ink }))
            .child(text.to_owned());
        if numeric { tabular(cell) } else { cell }
    }
    fn account_cell(&self, provider: Provider, width: f32) -> Div {
        div()
            .w(px(width))
            .min_w(px(0.))
            .flex_shrink_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(MARK_GAP))
            .child(dot(provider, self.config.dark))
            .child(
                self.table_cell(provider.name(), None, false, false)
                    .flex_1()
                    .min_w(px(0.)),
            )
    }
    /// Model name, with the account dot inline when the account column is hidden.
    fn model_cell(&self, model: &str, provider: Option<Provider>, width: f32) -> Div {
        div()
            .w(px(width))
            .min_w(px(0.))
            .flex_shrink_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(MARK_GAP))
            .when_some(provider, |cell, provider| {
                cell.child(dot(provider, self.config.dark))
            })
            .child(
                self.table_cell(model, None, false, false)
                    .flex_1()
                    .min_w(px(0.)),
            )
    }
    fn table_header(&self) -> Div {
        let theme = self.theme();
        div()
            .w_full()
            .h(px(TABLE_HEADER))
            .flex_shrink_0()
            .px(px(TABLE_PAD))
            .flex()
            .items_center()
            .gap(px(COLUMN_GAP))
            .bg(rgb(theme.bg))
            .border_b_1()
            .border_color(rgb(theme.line))
    }
    fn table_row(&self, index: usize) -> Div {
        let theme = self.theme();
        div()
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(TABLE_PAD))
            .flex()
            .items_center()
            .gap(px(COLUMN_GAP))
            .when(index > 0, |row| {
                row.border_t_1().border_color(rgb(theme.grid))
            })
    }
    /// Count on the left, then "Reset sort" when the order was changed.
    fn table_footer(
        &self,
        table: Table,
        count: String,
        estimates: bool,
        cx: &Context<Self>,
    ) -> Div {
        let theme = self.theme();
        div()
            .h(px(TABLE_FOOTER))
            .flex_shrink_0()
            .px(px(TABLE_PAD))
            .flex()
            .items_center()
            .gap(px(SPACE_XS))
            .border_t_1()
            .border_color(rgb(theme.line))
            .child(tabular(caption(count, theme)))
            .when(!self.sort_of(table).is_default(), |row| {
                row.child(self.text_action(
                    format!("reset-sort-{table:?}"),
                    "Reset sort",
                    Some("reset"),
                    UiAction::ResetSort(table),
                    cx,
                ))
            })
            .when(estimates, |row| {
                row.child(caption("~ estimated at API rates", theme))
            })
    }
    pub(super) fn account_select(&self, id: &str, cx: &Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let provider = self.provider;
        let options = std::iter::once(None)
            .chain(self.visible_accounts().iter().map(|s| Some(s.provider)))
            .collect::<Vec<_>>();
        self.select(
            format!("account-filter-{id}"),
            provider.map(Provider::name).unwrap_or("All accounts"),
            136.,
            provider.is_some(),
            cx,
        )
        .dropdown_menu(move |mut menu, _, _| {
            for option in &options {
                let option = *option;
                let view = weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(option.map(Provider::name).unwrap_or("All accounts"))
                        .checked(provider == option)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.perform(UiAction::Provider(option), window, cx)
                            });
                        }),
                );
            }
            menu
        })
    }
    fn table_filters(
        &self,
        table: Table,
        width: f32,
        extras: Vec<AnyElement>,
        cx: &Context<Self>,
    ) -> Div {
        use gpui_component::{Sizable, input::Input};
        let query = match table {
            Table::Models => &self.model_query,
            Table::Activity | Table::Projects => &self.activity_query,
        };
        let cost_filter = match table {
            Table::Models => self.model_cost,
            Table::Activity | Table::Projects => self.activity_cost,
        };
        let weak = cx.weak_entity();
        let costs = self
            .select(
                format!("cost-filter-{table:?}"),
                cost_filter.label(),
                116.,
                cost_filter != CostFilter::All,
                cx,
            )
            .dropdown_menu(move |mut menu, _, _| {
                for option in CostFilter::ALL {
                    let view = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(option.label())
                            .checked(cost_filter == option)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |view, cx| {
                                    view.perform(UiAction::CostFilter(table, option), window, cx)
                                });
                            }),
                    );
                }
                menu
            });
        let active = self.provider.is_some()
            || cost_filter != CostFilter::All
            || !query.read(cx).value().is_empty();
        let clear = {
            use gpui_component::Disableable;
            self.button(
                format!("clear-filters-{table:?}"),
                "Clear",
                None,
                false,
                UiAction::ClearFilters(table),
                cx,
            )
            .w(px(60.))
            .disabled(!active)
        };
        let controls = div()
            .flex()
            .items_center()
            .gap(px(SPACE_XS))
            .child(self.account_select(&format!("{table:?}"), cx))
            .child(costs)
            .child(clear);
        let search: gpui::AnyElement = if table == Table::Activity {
            self.activity_search().into_any_element()
        } else {
            Input::new(query)
                .small()
                .h(px(CONTROL_HEIGHT))
                .cleanable(true)
                .prefix(gpui_component::IconName::Search)
                .flex_1()
                .min_w(px(160.))
                .into_any_element()
        };
        let body = div().flex_shrink_0().flex().flex_col().gap(px(SPACE_XS));
        if width >= 650. {
            body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(search)
                    .child(controls)
                    .children(extras),
            )
        } else {
            body.child(div().flex().child(search))
                .child(controls.child(div().flex_1()).children(extras))
        }
    }
    /// Activity search, sized and colored like the filters beside it.
    /// Placeholder and clear stay on the existing input state.
    fn activity_search(&self) -> gpui_component::input::Input {
        use gpui_component::{Sizable, input::Input};
        let theme = self.theme();
        Input::new(&self.activity_query)
            .small()
            .h(px(CONTROL_HEIGHT))
            .py(px(0.))
            .pl(px(SPACE_S))
            .pr(px(SPACE_XS))
            .bg(rgb(theme.panel))
            .text_size(px(TYPE_CAPTION))
            .text_color(rgb(theme.ink))
            .rounded(px(RADIUS))
            .cleanable(true)
            .prefix(
                icon("search", theme)
                    .size(px(ICON_BODY))
                    .text_color(rgb(theme.muted)),
            )
            .flex_1()
            .min_w(px(180.))
    }

    // ----- Accounts -------------------------------------------------------

    pub(super) fn overview(&self, width: f32, height: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let analysis = self.analysis_or_empty(self.derived.overview.value(), self.period, None);
        let accounts = self.visible_accounts();
        let layout = layout::OverviewLayout::new(
            width,
            height,
            accounts.len(),
            accounts
                .iter()
                .map(|status| status.limits.len())
                .max()
                .unwrap_or(0),
        );
        let body = div().size_full().flex().flex_col().gap(px(layout.gap));
        if accounts.is_empty() {
            return body.child(
                div().flex_1().flex().items_center().justify_center().child(
                    self.empty_block("No accounts found", None)
                        .child(self.button(
                            "empty-connect",
                            "Connections",
                            Some("connections"),
                            false,
                            UiAction::Page(Page::Connections),
                            cx,
                        )),
                ),
            );
        }
        let totals = &analysis.totals;
        let cost_note = (layout.width >= 800.).then(|| self.cost_caption(totals));
        let models = analysis.models.len() as u64;
        let summary = div()
            .h(px(layout.summary_height))
            .flex_shrink_0()
            .flex()
            .pb(px(if layout.dense { SPACE_XS } else { SPACE_S }))
            .child(self.summary_figure(
                "Cost this period",
                self.spend(totals),
                cost_note,
                true,
                layout.dense,
            ))
            .child(self.summary_figure(
                "Tokens",
                self.number(totals.tokens.total()),
                None,
                false,
                layout.dense,
            ))
            .child(self.summary_figure(
                "Sessions",
                self.number(totals.sessions as u64),
                None,
                false,
                layout.dense,
            ))
            .when(layout.width >= 560., |row| {
                row.child(self.summary_figure(
                    "Models",
                    self.number(models),
                    None,
                    false,
                    layout.dense,
                ))
            });
        let heading = div()
            .h(px(TEXT_ACTION_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(label("Allowances", theme))
            .child(div().flex_1())
            .child(self.text_action(
                "all-limits",
                "All limits",
                None,
                UiAction::Page(Page::Limits),
                cx,
            ));
        let grid = div()
            .id("home-accounts")
            .h(px(layout.accounts_height))
            .flex_shrink_0()
            .when(layout.scroll_accounts, |grid| grid.overflow_y_scroll())
            .child(div().flex().flex_col().gap(px(layout.gap)).children(
                accounts.chunks(layout.columns).map(|row| {
                    div()
                        .flex()
                        .gap(px(layout.gap))
                        .children(row.iter().map(|status| {
                            self.account_card(
                                status,
                                &analysis.providers[status.provider.index()],
                                layout,
                                cx,
                            )
                        }))
                }),
            ));
        body.child(summary)
            .child(heading)
            .child(grid)
            .child(self.usage_chart(&analysis, layout.chart_height, layout.width - 32., cx))
    }

    fn account_card(
        &self,
        status: &ProviderStatus,
        totals: &Totals,
        layout: layout::OverviewLayout,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let theme = self.theme();
        let provider = status.provider;
        let has_history = status.sources > 0
            || (provider == Provider::Cursor && status.connected)
            || self.has_records[provider.index()];
        let tokens = if has_history {
            self.number(totals.tokens.total())
        } else {
            analytics::empty_value().into()
        };
        let reading = if !status.stale && status.checked_at.is_some() {
            "Live"
        } else if status.checked_at.is_some() {
            "Cached"
        } else if status.connected {
            "Local"
        } else {
            "Detected"
        };
        let plan = status.plan.as_deref().map(plan_name).unwrap_or_default();
        let extras = status.limits.len().saturating_sub(layout.limits);
        let subtitle = [plan.as_str(), reading]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(analytics::separator());
        let figure = |name: &str, value: String| {
            div()
                .flex_1()
                .min_w_0()
                .child(caption(name.to_owned(), theme))
                .child(
                    tabular(div())
                        .text_size(px(TYPE_TITLE))
                        .line_height(relative(1.15))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(value),
                )
        };
        let header = div()
            .h(px(if layout.cards { 40. } else { 24. }))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(SPACE_S))
            .child(
                provider_mark(provider, self.config.dark).size(px(if layout.cards {
                    28.
                } else {
                    22.
                })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(relative(1.2))
                    .child(
                        div()
                            .text_size(px(if layout.cards { TYPE_TITLE } else { TYPE_BODY }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(provider.account_name()),
                    )
                    .when(layout.cards, |column| {
                        column.child(caption(subtitle.clone(), theme).truncate())
                    }),
            )
            .when(extras > 0, |row| {
                row.child(caption(format!("+{extras} more"), theme).flex_shrink_0())
            })
            .when(!layout.cards, |row| {
                row.child(
                    div()
                        .flex()
                        .gap(px(SPACE_S))
                        .text_size(px(TYPE_CAPTION))
                        .child(
                            tabular(div())
                                .w(px(60.))
                                .text_align(TextAlign::Right)
                                .text_color(rgb(theme.muted))
                                .child(tokens.clone()),
                        )
                        .child(
                            tabular(div())
                                .w(px(64.))
                                .text_align(TextAlign::Right)
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.spend(totals)),
                        ),
                )
            });
        let allowances = div()
            .flex()
            .gap(px(if layout.cards { 0. } else { 14. }))
            .when(layout.cards, |row| row.flex_col())
            .when(status.limits.is_empty(), |row| {
                row.items_center().child(caption(
                    if status.detected && !status.connected {
                        "Sign in to see allowances"
                    } else {
                        "No allowance reported"
                    },
                    theme,
                ))
            })
            .children(status.limits.iter().take(layout.limits).map(|limit| {
                div()
                    .when(!layout.cards, |cell| cell.flex_1().min_w_0())
                    .when(layout.cards, |cell| {
                        cell.h(px(48.)).flex_shrink_0().pt(px(6.))
                    })
                    .child(self.account_quota(limit, status.stale, provider, true))
            }));
        let tip = format!("{subtitle}\n{}", self.cost_caption(totals));
        panel(theme)
            .id(SharedString::from(format!("account-{}", provider.key())))
            .w(px(layout.item_width()))
            .h(px(layout.item_height))
            .flex_shrink_0()
            .p(px(if layout.cards { 16. } else { 6. }))
            .px(px(if layout.cards { 16. } else { 12. }))
            .flex()
            .flex_col()
            .cursor_pointer()
            .hover(move |style| style.border_color(rgb(theme.faint)))
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.perform(UiAction::AccountActivity(provider), window, cx)
            }))
            .child(header)
            .child(
                allowances
                    .flex_1()
                    .min_h_0()
                    .when(!layout.cards, |row| row.mt(px(2.))),
            )
            .when(layout.cards, |card| {
                card.child(
                    div()
                        .h(px(50.))
                        .flex_shrink_0()
                        .pt(px(SPACE_XS))
                        .border_t_1()
                        .border_color(rgb(theme.grid))
                        .flex()
                        .gap(px(SPACE_S))
                        .child(figure("Tokens", tokens.clone()))
                        .child(figure("Cost", self.spend(totals))),
                )
            })
    }

    fn meter_color(&self, limit: &LimitWindow, stale: bool, provider: Provider) -> u32 {
        let theme = self.theme();
        if limit.expired(Utc::now()) || stale {
            theme.faint
        } else if limit.remaining() <= 10. {
            theme.danger
        } else if limit.remaining() <= 25. {
            theme.warn
        } else {
            color(provider, self.config.dark)
        }
    }
    /// A gauge of short ticks rather than a rounded bar: remaining allowance
    /// reads like a fuel gauge, and partial ticks fill proportionally.
    fn meter(&self, limit: &LimitWindow, color: u32, height: f32) -> Div {
        const TICKS: usize = 24;
        let theme = self.theme();
        let filled = if self.config.privacy || limit.expired(Utc::now()) {
            0.
        } else {
            (limit.remaining() as f32 / 100.).clamp(0., 1.) * TICKS as f32
        };
        div()
            .h(px(height))
            .w_full()
            .flex()
            .gap(px(2.))
            .children((0..TICKS).map(|tick| {
                let share = (filled - tick as f32).clamp(0., 1.);
                div()
                    .flex_1()
                    .h_full()
                    .rounded(px(1.))
                    .bg(rgb(theme.wash))
                    .overflow_hidden()
                    .when(share > 0., |cell| {
                        cell.child(div().h_full().w(relative(share)).bg(rgb(color)))
                    })
            }))
    }
    fn remaining_text(&self, limit: &LimitWindow, long: bool) -> String {
        if self.config.privacy {
            "Hidden".into()
        } else if limit.expired(Utc::now()) {
            "Refresh needed".into()
        } else if let Some(credits) = &limit.credits {
            format!(
                "{:.2}{} left",
                (credits.limit - credits.used).max(0.),
                if long { " credits" } else { "" }
            )
        } else {
            format!("{:.0}% left", limit.remaining())
        }
    }
    fn account_quota(
        &self,
        limit: &LimitWindow,
        stale: bool,
        provider: Provider,
        dense: bool,
    ) -> Div {
        let theme = self.theme();
        let expired = limit.expired(Utc::now());
        let warning = !expired && !stale && limit.remaining() <= 25. && !self.config.privacy;
        let bar_color = self.meter_color(limit, stale, provider);
        let reset = if self.config.privacy {
            "Reset hidden".into()
        } else {
            reset_text(limit.resets_at)
        };
        div()
            .h(px(if dense { 40. } else { 56. }))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(if dense { 2. } else { 5. }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .text_size(px(TYPE_CAPTION))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(theme.muted))
                            .child(limit.label.clone()),
                    )
                    .child(
                        tabular(div())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(if warning { bar_color } else { theme.ink }))
                            .child(self.remaining_text(limit, false)),
                    ),
            )
            .child(self.meter(limit, bar_color, 5.))
            .child(caption(reset, theme).truncate())
    }
    fn quota(&self, limit: &LimitWindow, stale: bool, provider: Provider) -> Div {
        let theme = self.theme();
        div()
            .flex()
            .flex_col()
            .gap(px(SPACE_XS))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(
                        label(limit.label.clone(), theme)
                            .flex_1()
                            .min_w_0()
                            .truncate(),
                    )
                    .child(
                        tabular(div())
                            .text_size(px(TYPE_BODY))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.remaining_text(limit, true)),
                    ),
            )
            .child(self.meter(limit, self.meter_color(limit, stale, provider), 6.))
            .child(caption(
                if self.config.privacy {
                    "Reset hidden".into()
                } else {
                    reset_text(limit.resets_at)
                },
                theme,
            ))
    }

    // ----- Models ---------------------------------------------------------

    pub(super) fn models_view(&self, width: f32, height: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let content_width = layout::content_width(width, height);
        let result = self.derived.models.value();
        let (shown, total, estimates) = result
            .as_ref()
            .map_or((0, 0, false), |r| (r.rows.len(), r.total, r.estimates));
        let columns = model_column_widths(content_width);
        let narrow = content_width < 650.;
        let count = if shown == total {
            format!("{total} models")
        } else {
            format!("{shown} of {total} models")
        };
        // Only the rows in view are built, however many models there are.
        let rows = {
            let columns = columns.clone();
            uniform_list(
                "model-table-rows",
                shown,
                cx.processor(move |view, range: std::ops::Range<usize>, _, _| {
                    let Some(result) = view.derived.models.value() else {
                        return vec![];
                    };
                    range
                        .filter_map(|index| {
                            result
                                .rows
                                .get(index)
                                .map(|row| view.model_row(row, index, &columns, narrow))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h_0()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(FILTER_GAP))
            .child(self.table_filters(Table::Models, content_width, vec![], cx))
            .child(
                panel(theme)
                    .p(px(0.))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        self.table_header()
                            .children(columns.iter().map(|(column, width)| {
                                self.sort_header(
                                    Table::Models,
                                    *column,
                                    Some(*width),
                                    column_align_end(*column),
                                    column_has_mark(*column, narrow),
                                    cx,
                                )
                            })),
                    )
                    .child(if shown == 0 {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p(px(SPACE_L))
                            .child(self.empty_block(
                                if total == 0 {
                                    "No model usage in this period"
                                } else {
                                    "No models match these filters"
                                },
                                None,
                            ))
                            .into_any_element()
                    } else {
                        rows.into_any_element()
                    })
                    .child(self.table_footer(Table::Models, count, estimates, cx)),
            )
    }
    fn model_row(
        &self,
        row: &token_usage::analytics::ModelRow,
        index: usize,
        columns: &[(Column, f32)],
        narrow: bool,
    ) -> Div {
        let totals = &row.totals;
        self.table_row(index)
            .children(columns.iter().map(|(column, width)| {
                match column {
                    Column::Model => self
                        .model_cell(&row.model, narrow.then_some(row.provider), *width)
                        .into_any_element(),
                    Column::Account => self.account_cell(row.provider, *width).into_any_element(),
                    _ => {
                        let text = match column {
                            Column::Tokens => self.number(totals.tokens.total()),
                            Column::Input => self.number(totals.tokens.input),
                            Column::Output => self.number(totals.tokens.output),
                            Column::CacheRead => self.number(totals.tokens.cache_read),
                            Column::CacheWrite => self.number(totals.tokens.cache_write),
                            Column::Sessions => self.number(totals.sessions as u64),
                            _ => self.cost_text(totals),
                        };
                        let muted = *column != Column::Tokens && *column != Column::Cost;
                        self.table_cell(&text, Some(*width), column_align_end(*column), muted)
                            .into_any_element()
                    }
                }
            }))
    }

    // ----- Limits ---------------------------------------------------------

    pub(super) fn limits_view(&self, width: f32, height: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let accounts = self.visible_accounts();
        let content_width = layout::content_width(width, height).min(1200.);
        let card_columns = if content_width >= 900. { 2 } else { 1 };
        let card_width =
            (content_width - SPACE_S * (card_columns - 1) as f32) / card_columns as f32;
        let meter_columns = ((card_width - SPACE_M * 2. + SPACE_L) / (220. + SPACE_L))
            .floor()
            .clamp(1., 3.) as usize;
        if accounts.is_empty() {
            return self.page_body().child(
                panel(theme)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(SPACE_M))
                    .child(title("No connected accounts", theme))
                    .child(self.button(
                        "limits-connect",
                        "Connections",
                        Some("connections"),
                        false,
                        UiAction::Page(Page::Connections),
                        cx,
                    )),
            );
        }
        self.page_body()
            .children(accounts.chunks(card_columns).map(|row| {
                div()
                    .flex()
                    .gap(px(SPACE_S))
                    .children(row.iter().map(|status| {
                        let reading = if !status.stale && status.checked_at.is_some() {
                            "Live"
                        } else if status.checked_at.is_some() {
                            "Cached"
                        } else {
                            "Not checked"
                        };
                        panel(theme)
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(SPACE_M))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(SPACE_S))
                                    .child(provider_mark(status.provider, self.config.dark))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(title(status.provider.account_name(), theme))
                                            .child(caption(
                                                status
                                                    .plan
                                                    .as_deref()
                                                    .map(plan_name)
                                                    .unwrap_or_else(|| "Plan not reported".into()),
                                                theme,
                                            )),
                                    )
                                    .child(self.badge(reading, reading == "Live")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(SPACE_M))
                                    .when(status.limits.is_empty(), |column| {
                                        column.child(caption("No allowances reported", theme))
                                    })
                                    .children(status.limits.chunks(meter_columns).map(|limits| {
                                        div()
                                            .flex()
                                            .gap(px(SPACE_L))
                                            .children(limits.iter().map(|limit| {
                                                div().flex_1().min_w_0().child(self.quota(
                                                    limit,
                                                    status.stale,
                                                    status.provider,
                                                ))
                                            }))
                                            .children(
                                                (limits.len()..meter_columns)
                                                    .map(|_| div().flex_1().min_w_0()),
                                            )
                                    })),
                            )
                            .child(div().flex_1())
                            .child(caption(
                                status
                                    .checked_at
                                    .map(|at| {
                                        format!(
                                            "Checked {}",
                                            at.with_timezone(&Local).format("%b %-d, %H:%M")
                                        )
                                    })
                                    .unwrap_or_else(|| "Not checked".into()),
                                theme,
                            ))
                    }))
                    .children((row.len()..card_columns).map(|_| div().flex_1()))
            }))
    }

    // ----- Activity -------------------------------------------------------

    pub(super) fn activity(&self, width: f32, viewport_height: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let content_width = layout::content_width(width, viewport_height);
        let result = self.derived.activity.value();
        let total = result.as_ref().map_or(0, |r| r.total);
        let matched = result.as_ref().map_or(0, |r| r.indices.len());
        let detail = self.detail.as_ref().and_then(|id| {
            result
                .as_ref()
                .and_then(|r| r.records().find(|record| record.id == *id))
        });
        let fitted = rows_that_fit(width, viewport_height, detail.is_some());
        let page_size = self.activity_rows.unwrap_or(fitted).max(1);
        let page_count = matched.div_ceil(page_size);
        let page = self.activity_page.min(page_count.saturating_sub(1));
        let start = page * page_size;
        let visible = result
            .as_ref()
            .map(|r| r.records().skip(start).take(page_size).collect::<Vec<_>>())
            .unwrap_or_default();
        let estimates = visible
            .iter()
            .any(|record| matches!(analytics::record_cost(record), Some((_, true))));
        let columns = activity_column_widths(content_width);
        let narrow = content_width < 650.;
        let count = if matched == 0 {
            if total == 0 {
                "0 events".to_string()
            } else {
                format!("0 of {} events", grouped(total))
            }
        } else {
            format!(
                "{}-{} of {}",
                start + 1,
                (start + page_size).min(matched),
                grouped(matched)
            )
        };
        let records_empty = matched == 0;
        let extras = vec![
            self.activity_row_options(cx).into_any_element(),
            self.icon_button(
                "activity-export",
                "export",
                "Export CSV",
                UiAction::Export,
                cx,
            )
            .into_any_element(),
        ];
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(FILTER_GAP))
            .child(self.table_filters(Table::Activity, content_width, extras, cx))
            .when_some(detail, |body, record| {
                body.child(
                    div()
                        .id("activity-detail")
                        .h(px(activity_detail_height(width, viewport_height)))
                        .flex_shrink_0()
                        .overflow_y_scroll()
                        .child(self.usage_detail(record, content_width, cx)),
                )
            })
            .child(
                panel(theme)
                    .p(px(0.))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        self.table_header()
                            .children(columns.iter().map(|(column, width)| {
                                self.sort_header(
                                    Table::Activity,
                                    *column,
                                    Some(*width),
                                    column_align_end(*column),
                                    column_has_mark(*column, narrow),
                                    cx,
                                )
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "activity-rows-{page}-{page_size}"
                            )))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .when(records_empty, |rows| {
                                rows.child(
                                    div()
                                        .h_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .p(px(SPACE_L))
                                        .child(self.empty_block(
                                            if total == 0 {
                                                "No usage events in this period"
                                            } else {
                                                "No events match these filters"
                                            },
                                            None,
                                        )),
                                )
                            })
                            .children(visible.iter().enumerate().map(|(index, record)| {
                                let id = record.id.clone();
                                let selected = self.detail.as_deref() == Some(id.as_str());
                                self.table_row(index)
                                    .id(SharedString::from(format!("record-{id}")))
                                    .cursor_pointer()
                                    .when(selected, |row| row.bg(rgb(theme.wash)))
                                    .hover(move |style| style.bg(rgb(theme.wash)))
                                    .children(columns.iter().map(|(column, width)| {
                                        self.activity_cell(record, *column, *width, narrow)
                                    }))
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.perform(UiAction::Detail(id.clone()), window, cx)
                                    }))
                            })),
                    )
                    .child(
                        self.table_footer(Table::Activity, count, estimates, cx)
                            .child(div().flex_1())
                            .when(page_count > 1, |row| {
                                row.child(self.activity_pager(page, page_count, narrow, cx))
                            }),
                    ),
            )
    }
    fn activity_cell(
        &self,
        record: &UsageRecord,
        column: Column,
        width: f32,
        narrow: bool,
    ) -> AnyElement {
        let time = |format: &str| {
            record
                .timestamp
                .with_timezone(&Local)
                .format(format)
                .to_string()
        };
        match column {
            Column::Account => self.account_cell(record.provider, width).into_any_element(),
            Column::Model => self
                .model_cell(&record.model, narrow.then_some(record.provider), width)
                .into_any_element(),
            Column::Time => self
                .table_cell(
                    &time(if narrow {
                        "%-m/%-d %H:%M"
                    } else {
                        "%b %-d, %H:%M"
                    }),
                    Some(width),
                    false,
                    true,
                )
                .into_any_element(),
            Column::Project => {
                let project = if self.config.privacy {
                    "Hidden"
                } else if record.project.is_empty() {
                    analytics::empty_value()
                } else {
                    &record.project
                };
                self.table_cell(project, Some(width), false, true)
                    .into_any_element()
            }
            Column::Tokens => self
                .table_cell(
                    &self.number(record.tokens.total()),
                    Some(width),
                    true,
                    false,
                )
                .into_any_element(),
            _ => {
                let text = match analytics::record_cost(record) {
                    Some((cost, true)) if !self.config.privacy => format!("~{}", self.money(cost)),
                    Some((cost, _)) => self.money(cost),
                    None => analytics::empty_value().into(),
                };
                self.table_cell(&text, Some(width), true, false)
                    .into_any_element()
            }
        }
    }
    fn activity_row_options(&self, cx: &Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let selected = self.activity_rows;
        self.select(
            "activity-row-count",
            selected
                .map(|n| format!("{n} rows"))
                .unwrap_or_else(|| "Fit rows".into()),
            100.,
            false,
            cx,
        )
        .dropdown_menu(move |mut menu, _, _| {
            for (limit, name) in [
                (None, "Fit window"),
                (Some(25), "25 rows"),
                (Some(50), "50 rows"),
                (Some(100), "100 rows"),
            ] {
                let view = weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(name)
                        .checked(selected == limit)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.perform(UiAction::ActivityRows(limit), window, cx)
                            });
                        }),
                );
            }
            menu
        })
    }
    fn activity_pager(
        &self,
        page: usize,
        page_count: usize,
        narrow: bool,
        cx: &Context<Self>,
    ) -> Div {
        let theme = self.theme();
        div()
            .flex()
            .items_center()
            .gap(px(SPACE_2XS))
            .child(self.page_step(
                "activity-prev",
                "Prev",
                page > 0,
                false,
                page.saturating_sub(1),
                cx,
            ))
            .children(
                (if narrow {
                    Vec::new()
                } else {
                    activity_pages(page, page_count)
                })
                .into_iter()
                .enumerate()
                .map(|(index, item)| match item {
                    Some(number) => self.page_step(
                        format!("activity-page-{number}"),
                        (number + 1).to_string(),
                        true,
                        number == page,
                        number,
                        cx,
                    ),
                    None => div()
                        .id(SharedString::from(format!("activity-gap-{index}")))
                        .w(px(TEXT_ACTION_HEIGHT))
                        .text_center()
                        .text_size(px(TYPE_CAPTION))
                        .text_color(rgb(theme.faint))
                        .child("..."),
                }),
            )
            .child(self.page_step(
                "activity-next",
                "Next",
                page + 1 < page_count,
                false,
                page + 1,
                cx,
            ))
    }
    fn page_step(
        &self,
        id: impl Into<String>,
        text: impl Into<String>,
        enabled: bool,
        current: bool,
        page: usize,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let theme = self.theme();
        let text = text.into();
        let wide = text.len() > 3;
        tabular(div())
            .id(SharedString::from(id.into()))
            .h(px(CONTROL_HEIGHT))
            .w(px(if wide { 48. } else { CONTROL_HEIGHT }))
            .rounded(px(RADIUS))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(TYPE_CAPTION))
            .text_color(rgb(if current || enabled {
                theme.ink
            } else {
                theme.faint
            }))
            .when(current, |chip| {
                chip.bg(rgb(theme.wash)).font_weight(FontWeight::MEDIUM)
            })
            .when(enabled && !current, |chip| {
                chip.cursor_pointer()
                    .hover(move |style| style.bg(rgb(theme.wash)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.perform(UiAction::ActivityPage(page), window, cx)
                    }))
            })
            .child(text)
    }
    fn usage_detail(&self, record: &UsageRecord, width: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let dense = width < 650.;
        let (amount, note) = self.event_cost(record);
        let value = |name: &str, value: String| {
            div()
                .flex_1()
                .min_w_0()
                .child(caption(name.to_owned(), theme))
                .child(
                    tabular(div())
                        .text_size(px(TYPE_TITLE))
                        .line_height(relative(1.15))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(value),
                )
        };
        let figures = [
            ("Input", self.number(record.tokens.input)),
            ("Output", self.number(record.tokens.output)),
            ("Cache read", self.number(record.tokens.cache_read)),
            ("Cache write", self.number(record.tokens.cache_write)),
            ("Cost", amount),
        ];
        panel(theme)
            .flex()
            .flex_col()
            .gap(px(SPACE_M))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_S))
                    .child(provider_mark(record.provider, self.config.dark).size(px(28.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(title(record.model.clone(), theme).truncate())
                            .child(
                                caption(
                                    [
                                        record.provider.name().to_string(),
                                        record
                                            .timestamp
                                            .with_timezone(&Local)
                                            .format("%b %-d, %H:%M:%S")
                                            .to_string(),
                                        note,
                                    ]
                                    .join(analytics::separator()),
                                    theme,
                                )
                                .truncate(),
                            ),
                    )
                    .child(self.icon_button(
                        "detail-close",
                        "close",
                        "Close",
                        UiAction::CloseDetail,
                        cx,
                    )),
            )
            .child(
                div().flex().flex_col().gap(px(SPACE_S)).children(
                    figures
                        .chunks(if dense { 3 } else { 5 })
                        .map(|row| {
                            div().flex().gap(px(SPACE_M)).children(
                                row.iter().map(|(name, figure)| value(name, figure.clone())),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            )
            .when(!self.config.privacy, |detail| {
                detail.child(
                    div()
                        .pt(px(SPACE_S))
                        .border_t_1()
                        .border_color(rgb(theme.grid))
                        .flex()
                        .flex_col()
                        .gap(px(SPACE_2XS))
                        .child(label(
                            if record.project.is_empty() {
                                "Project not reported".to_string()
                            } else {
                                record.project.clone()
                            },
                            theme,
                        ))
                        .child(caption(record.source.clone(), theme).truncate()),
                )
            })
    }

    // ----- Connections ----------------------------------------------------

    pub(super) fn connections(&self, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        div().w_full().max_w(px(760.)).mx_auto().child(
            panel(theme).p(px(0.)).overflow_hidden().children(
                Provider::ALL
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, provider)| {
                        self.data()
                            .providers
                            .get(provider.index())
                            .filter(|status| status.provider == provider)
                            .map(|status| self.connection_account(status, index > 0, cx))
                    }),
            ),
        )
    }
    fn connection_account(
        &self,
        status: &ProviderStatus,
        divider: bool,
        cx: &Context<Self>,
    ) -> Div {
        let theme = self.theme();
        let provider = status.provider;
        let included = self.config.enabled[provider.index()];
        let summary = self.connection_summary(status);
        let sources = self.listed_sources(status);
        div()
            .when(divider, |row| {
                row.border_t_1().border_color(rgb(theme.line))
            })
            .px(px(SPACE_M))
            .py(px(SPACE_M))
            .flex()
            .items_start()
            .gap(px(SPACE_S))
            .child(provider_mark(provider, self.config.dark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_XS))
                    .child(
                        div()
                            .min_h(px(CONTROL_HEIGHT))
                            .flex()
                            .flex_col()
                            .justify_center()
                            .gap(px(SPACE_2XS))
                            .child(title(provider.account_name(), theme))
                            .when_some(summary.filter(|_| included), |column, summary| {
                                column.child(caption(summary, theme).truncate())
                            }),
                    )
                    .when(included, |column| {
                        column
                            .children(sources.iter().map(|source| {
                                let remove = source.remove.clone();
                                div()
                                    .h(px(CONTROL_HEIGHT))
                                    .pl(px(SPACE_S))
                                    .rounded(px(RADIUS))
                                    .bg(rgb(theme.bg))
                                    .flex()
                                    .items_center()
                                    .gap(px(SPACE_XS))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_family(mono_font())
                                            .text_size(px(TYPE_CAPTION))
                                            .text_color(rgb(theme.muted))
                                            .child(source.text.clone()),
                                    )
                                    .when_some(remove, |row, (id, action)| {
                                        row.child(
                                            self.icon_button(id, "close", "Remove", action, cx),
                                        )
                                    })
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(SPACE_XS))
                                    .child(self.button(
                                        format!("add-home-{}", provider.key()),
                                        "Add folder",
                                        None,
                                        false,
                                        UiAction::AddHome(provider),
                                        cx,
                                    ))
                                    .when(provider == Provider::Cursor, |row| {
                                        row.child(self.button(
                                            "cursor-csv",
                                            "Import CSV",
                                            None,
                                            false,
                                            UiAction::ImportCursor,
                                            cx,
                                        ))
                                    })
                                    .child(div().flex_1())
                                    .child(self.text_action(
                                        format!("open-account-{}", provider.key()),
                                        "Dashboard",
                                        Some("arrow"),
                                        UiAction::OpenProvider(provider),
                                        cx,
                                    )),
                            )
                    }),
            )
            .child(
                div()
                    .h(px(CONTROL_HEIGHT))
                    .flex()
                    .items_center()
                    .child(self.toggle(
                        &format!("include-{}", provider.key()),
                        included,
                        move |view, on, _, cx| {
                            view.config.enabled[provider.index()] = on;
                            view.save(cx);
                            view.refresh(cx);
                        },
                        cx,
                    )),
            )
    }
    fn toggle(
        &self,
        id: &str,
        on: bool,
        change: impl Fn(&mut Self, bool, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let theme = self.theme();
        let change = std::rc::Rc::new(change);
        let change_key = change.clone();
        let (track, knob) = if on {
            (theme.ink, theme.panel)
        } else {
            (theme.line, theme.panel)
        };
        div()
            .id(SharedString::from(id.to_owned()))
            .focusable()
            .tab_index(0)
            .w(px(36.))
            .h(px(20.))
            .flex_shrink_0()
            .rounded_full()
            .cursor_pointer()
            .flex()
            .items_center()
            .px(px(2.))
            .border_1()
            .border_color(rgb(track))
            .bg(rgb(track))
            .when(on, |track| track.justify_end())
            .focus(move |style| style.border_color(rgb(theme.focus)))
            .on_click(cx.listener(move |view, _, window, cx| {
                change(view, !on, window, cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    change_key(view, !on, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(div().size(px(14.)).rounded_full().bg(rgb(knob)))
    }
    fn connection_summary(&self, status: &ProviderStatus) -> Option<String> {
        let mut parts = Vec::new();
        if status.provider == Provider::Cursor && !self.config.cursor_imports.is_empty() {
            parts.push("CSV".into());
        } else if status.provider != Provider::Cursor && status.sources > 0 {
            parts.push(if status.sources == 1 {
                "1 file".into()
            } else {
                format!("{} files", status.sources)
            });
        }
        if let Some(allowance) = self.allowance_label(status) {
            parts.push(allowance);
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(analytics::separator()))
        }
    }
    fn allowance_label(&self, status: &ProviderStatus) -> Option<String> {
        if !self.config.online {
            return Some(if status.checked_at.is_some() {
                "Cached".into()
            } else {
                "Paused".into()
            });
        }
        if !status.stale && status.checked_at.is_some() {
            return Some(
                status
                    .plan
                    .as_deref()
                    .map(plan_name)
                    .unwrap_or_else(|| "Live".into()),
            );
        }
        if reported_error(&status.detail) {
            return Some(status.detail.clone());
        }
        if status.checked_at.is_some() {
            return Some("Cached".into());
        }
        if status.detected {
            return Some("No sign-in".into());
        }
        None
    }
    fn listed_sources(&self, status: &ProviderStatus) -> Vec<ListedSource> {
        let provider = status.provider;
        let mut listed = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (index, path) in self.config.custom_homes[provider.index()]
            .iter()
            .enumerate()
        {
            seen.insert(path_key(path));
            listed.push(ListedSource {
                text: self.location_text(path),
                remove: Some((
                    format!("remove-home-{}-{index}", provider.key()),
                    UiAction::RemoveHome(provider, index),
                )),
            });
        }
        if provider == Provider::Cursor
            && let Some(path) = self.config.cursor_imports.first()
        {
            seen.insert(path_key(path));
            listed.push(ListedSource {
                text: self.location_text(path),
                remove: Some(("cursor-clear".into(), UiAction::ClearCursor)),
            });
        }
        let cached_homes = self.derived.homes.value();
        let defaults = cached_homes
            .as_ref()
            .and_then(|homes| homes.get(provider.index()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        for path in defaults {
            if !seen.insert(path_key(path)) {
                continue;
            }
            listed.push(ListedSource {
                text: self.location_text(path),
                remove: None,
            });
        }
        listed
    }
    fn location_text(&self, path: &std::path::Path) -> String {
        if self.config.privacy {
            "Hidden".into()
        } else {
            display_path(path)
        }
    }

    // ----- Settings -------------------------------------------------------

    pub(super) fn settings(&self, width: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let narrow = width < 700.;
        let intervals = token_usage::config::REFRESH_INTERVALS
            .into_iter()
            .map(|seconds| {
                Segment::new(
                    if seconds >= 3600 {
                        format!("{}h", seconds / 3600)
                    } else {
                        format!("{}m", seconds / 60)
                    },
                    self.config.refresh_seconds == seconds,
                    UiAction::Interval(seconds),
                )
            })
            .collect();
        let tray = if self.tray.is_some() {
            "Active"
        } else if self.tray_attempted {
            "Unavailable"
        } else {
            "Starting"
        };
        let section = |name: &str| {
            panel(theme).p(px(0.)).child(
                div()
                    .px(px(SPACE_M))
                    .pt(px(SPACE_M))
                    .pb(px(SPACE_XS))
                    .child(title(name.to_owned(), theme)),
            )
        };
        div()
            .w_full()
            .max_w(px(760.))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(SPACE_S))
            .child(
                section("Display")
                    .child(
                        self.preference_row(
                            "Appearance",
                            "",
                            self.segmented(
                                "appearance",
                                vec![
                                    Segment::new(
                                        "Light",
                                        !self.config.dark,
                                        if self.config.dark {
                                            UiAction::Dark
                                        } else {
                                            UiAction::Noop
                                        },
                                    ),
                                    Segment::new(
                                        "Dark",
                                        self.config.dark,
                                        if self.config.dark {
                                            UiAction::Noop
                                        } else {
                                            UiAction::Dark
                                        },
                                    ),
                                ],
                                64.,
                                cx,
                            )
                            .into_any_element(),
                        ),
                    )
                    .child(
                        self.preference_row(
                            "Privacy mode",
                            "Hide amounts, tokens, projects and paths",
                            self.toggle(
                                "privacy-toggle",
                                self.config.privacy,
                                |view, _, window, cx| view.perform(UiAction::Privacy, window, cx),
                                cx,
                            )
                            .into_any_element(),
                        ),
                    ),
            )
            .child(
                section("Data").child(
                    self.preference_row(
                        "Network access",
                        "Allow provider account checks and model price downloads",
                        self.toggle(
                            "online-toggle",
                            self.config.online,
                            |view, _, window, cx| view.perform(UiAction::Online, window, cx),
                            cx,
                        ).into_any_element(),
                    ),
                ).child(
                    self.preference_row(
                        "Automatic refresh",
                        if narrow {
                            ""
                        } else {
                            "Costs are reported or estimated at API rates and exclude subscriptions"
                        },
                        self.segmented("interval", intervals, 44., cx)
                            .into_any_element(),
                    ),
                ),
            )
            .child(
                section(if cfg!(target_os = "macos") {
                    "Menu bar"
                } else {
                    "System tray"
                })
                .child(
                    self.preference_row(
                        tray,
                        self.tray_error
                            .as_deref()
                            .unwrap_or(if self.tray.is_some() {
                                if cfg!(target_os = "macos") {
                                    "Closing hides to the menu bar"
                                } else {
                                    "Closing hides to the system tray"
                                }
                            } else {
                                "Closing the window quits the app"
                            }),
                        div()
                            .flex()
                            .gap(px(SPACE_XS))
                            .child(self.button(
                                "settings-quick",
                                "Quick view",
                                Some("overview"),
                                false,
                                UiAction::Compact,
                                cx,
                            ))
                            .child(self.button(
                                "settings-quit",
                                "Quit",
                                None,
                                false,
                                UiAction::Quit,
                                cx,
                            ))
                            .into_any_element(),
                    ),
                ),
            )
            .child(section("Updates").child({
                let note = if let Some(version) = &self.config.update_delayed {
                    format!("{version} delayed. The next launch asks again.")
                } else {
                    "Checks the latest stable GitHub release".into()
                };
                self.preference_row(
                    &format!("Version {}", token_usage::update::current_version()),
                    &note,
                    self.button(
                        "settings-update",
                        if self.update_checking {
                            "Checking"
                        } else {
                            "Check for updates"
                        },
                        None,
                        false,
                        if self.update_checking {
                            UiAction::Noop
                        } else {
                            UiAction::CheckUpdate
                        },
                        cx,
                    )
                    .into_any_element(),
                )
            }))
    }
    pub(super) fn update_banner(&self, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let text = if self.update_busy {
            "Downloading the update.".to_owned()
        } else if let Some(offer) = &self.update_offer {
            format!("Version {} is available.", offer.version)
        } else {
            "A new version is available.".to_owned()
        };
        div()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .right(px(0.))
            .px(px(SPACE_M))
            .py(px(SPACE_S))
            .bg(rgb(theme.ink))
            .text_color(rgb(theme.panel))
            .flex()
            .items_center()
            .gap(px(SPACE_S))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(TYPE_BODY))
                    .child(text),
            )
            .when(!self.update_busy, |bar| {
                bar.child(self.button(
                    "update-now",
                    "Update now",
                    None,
                    false,
                    UiAction::UpdateNow,
                    cx,
                ))
                .child(self.button(
                    "update-delay",
                    "Delay",
                    None,
                    false,
                    UiAction::UpdateDelay,
                    cx,
                ))
            })
    }
    fn preference_row(&self, name: &str, description: &str, control: AnyElement) -> Div {
        let theme = self.theme();
        div()
            .min_h(px(56.))
            .px(px(SPACE_M))
            .py(px(SPACE_S))
            .border_t_1()
            .border_color(rgb(theme.grid))
            .flex()
            .items_center()
            .gap(px(SPACE_L))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_2XS))
                    .child(div().text_size(px(TYPE_BODY)).child(name.to_owned()))
                    .when(!description.is_empty(), |text| {
                        text.child(caption(description.to_owned(), theme))
                    }),
            )
            .child(control)
    }

    // ----- Quick view -----------------------------------------------------

    pub(super) fn compact_view(&self, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let analysis = self.analysis_or_empty(self.derived.today.value(), Period::Today, None);
        let accounts = self.visible_accounts();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.bg))
            .child(
                div()
                    .h(px(layout::HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px(px(SPACE_M))
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .border_b_1()
                    .border_color(rgb(theme.line))
                    .child(title("Today", theme))
                    .child(tabular(caption(
                        format!("{} tokens", self.number(analysis.totals.tokens.total())),
                        theme,
                    )))
                    .child(div().flex_1())
                    .child(self.icon_button(
                        "compact-refresh",
                        "refresh",
                        "Refresh",
                        UiAction::Refresh,
                        cx,
                    ))
                    .child(self.icon_button(
                        "compact-full",
                        "arrow",
                        "Open dashboard",
                        UiAction::Page(Page::Overview),
                        cx,
                    )),
            )
            .child(
                div()
                    .id("quick-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(SPACE_S))
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_S))
                    .when(accounts.is_empty(), |body| {
                        body.child(
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(self.empty_block("No connected accounts", None)),
                        )
                    })
                    .children(accounts.iter().map(|status| {
                        let totals = &analysis.providers[status.provider.index()];
                        panel(theme)
                            .p(px(SPACE_M))
                            .flex()
                            .flex_col()
                            .gap(px(SPACE_S))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(SPACE_S))
                                    .child(
                                        provider_mark(status.provider, self.config.dark)
                                            .size(px(28.)),
                                    )
                                    .child(
                                        title(status.provider.account_name(), theme)
                                            .flex_1()
                                            .min_w_0()
                                            .truncate(),
                                    )
                                    .child(tabular(caption(
                                        format!("{} tokens", self.number(totals.tokens.total())),
                                        theme,
                                    )))
                                    .child(
                                        tabular(div())
                                            .w(px(64.))
                                            .text_align(TextAlign::Right)
                                            .text_size(px(TYPE_BODY))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(self.spend(totals)),
                                    ),
                            )
                            .children(
                                status
                                    .limits
                                    .iter()
                                    .take(2)
                                    .map(|limit| self.quota(limit, status.stale, status.provider)),
                            )
                    })),
            )
            .child(
                div()
                    .h(px(TABLE_HEADER))
                    .flex_shrink_0()
                    .px(px(SPACE_M))
                    .border_t_1()
                    .border_color(rgb(theme.line))
                    .flex()
                    .items_center()
                    .child(caption(
                        if self.refreshing {
                            "Refreshing...".into()
                        } else {
                            self.data()
                                .refreshed_at
                                .map(|at| {
                                    format!("Updated {}", at.with_timezone(&Local).format("%H:%M"))
                                })
                                .unwrap_or_else(|| "Not synced".into())
                        },
                        theme,
                    )),
            )
    }
}

/// Right-aligned columns. Time stays with the text columns.
fn column_align_end(column: Column) -> bool {
    matches!(
        column,
        Column::Tokens
            | Column::Input
            | Column::Output
            | Column::CacheRead
            | Column::CacheWrite
            | Column::Sessions
            | Column::Events
            | Column::Cost
    )
}
/// Account always draws a provider dot. Narrow tables fold that dot into Model.
fn column_has_mark(column: Column, narrow: bool) -> bool {
    column == Column::Account || (column == Column::Model && narrow)
}
/// Width inside the table padding. Header and rows both spend this on columns.
fn table_track(content_width: f32, columns: usize) -> f32 {
    let chrome = TABLE_BORDER * 2. + TABLE_PAD * 2. + COLUMN_GAP * columns.saturating_sub(1) as f32;
    (content_width - chrome).max(0.)
}
/// One width per column. `None` in `specs` shares whatever the fixed columns
/// leave, so the header and the body cannot pick different x positions.
fn resolve_columns(content_width: f32, specs: &[(Column, Option<f32>)]) -> Vec<(Column, f32)> {
    let track = table_track(content_width, specs.len());
    let fixed: f32 = specs.iter().filter_map(|(_, width)| *width).sum();
    let flexes = specs.iter().filter(|(_, width)| width.is_none()).count();
    let share = if flexes == 0 {
        0.
    } else {
        ((track - fixed) / flexes as f32).max(48.)
    };
    specs
        .iter()
        .map(|(column, width)| (*column, width.unwrap_or(share)))
        .collect()
}
/// Every Models column, including the flexible name, in header and row order.
fn model_column_widths(width: f32) -> Vec<(Column, f32)> {
    let fixed: &[(Column, f32)] = if width < 650. {
        &[(Column::Tokens, 72.), (Column::Cost, 80.)]
    } else if width < 1000. {
        &[
            (Column::Account, 100.),
            (Column::Tokens, 84.),
            (Column::CacheRead, 88.),
            (Column::Cost, 88.),
        ]
    } else {
        &[
            (Column::Account, 100.),
            (Column::Tokens, 84.),
            (Column::Input, 76.),
            (Column::Output, 76.),
            (Column::CacheRead, 88.),
            (Column::CacheWrite, 88.),
            (Column::Sessions, 72.),
            (Column::Cost, 88.),
        ]
    };
    let mut specs = vec![(Column::Model, None)];
    specs.extend(
        fixed
            .iter()
            .copied()
            .map(|(column, width)| (column, Some(width))),
    );
    resolve_columns(width, &specs)
}
/// Every Activity column, in header and row order.
fn activity_column_widths(width: f32) -> Vec<(Column, f32)> {
    let specs = if width < 650. {
        vec![
            (Column::Time, Some(80.)),
            (Column::Model, None),
            (Column::Tokens, Some(64.)),
            (Column::Cost, Some(72.)),
        ]
    } else if width < 900. {
        vec![
            (Column::Time, Some(112.)),
            (Column::Account, Some(100.)),
            (Column::Model, None),
            (Column::Tokens, Some(84.)),
            (Column::Cost, Some(88.)),
        ]
    } else {
        vec![
            (Column::Time, Some(112.)),
            (Column::Account, Some(100.)),
            (Column::Model, Some((width * 0.26).clamp(180., 320.))),
            (Column::Project, None),
            (Column::Tokens, Some(84.)),
            (Column::Cost, Some(88.)),
        ]
    };
    resolve_columns(width, &specs)
}
struct ListedSource {
    text: String,
    remove: Option<(String, UiAction)>,
}
fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text
    }
}
fn path_key(path: &std::path::Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}
fn reported_error(detail: &str) -> bool {
    let sep = analytics::separator();
    let known = [
        "Paused in Connections".to_string(),
        format!("Account history{sep}across your machines"),
        "Local history + live account limits".to_string(),
        "Live account limits".to_string(),
        "Cached Cursor account history".to_string(),
        format!("Detected on this device{sep}no local usage yet"),
        "Import a usage CSV or enable account checks".to_string(),
        "Connect a source to see usage".to_string(),
    ];
    if detail.is_empty()
        || known.iter().any(|item| item == detail)
        || detail.ends_with(&format!("history files{sep}local"))
    {
        return false;
    }
    !(detail.starts_with("Use ") && detail.contains("to create local history"))
}
fn plan_name(plan: &str) -> String {
    plan.split(['_', '-'])
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| format!("{}{}", first.to_uppercase(), chars.as_str()))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn activity_chrome(width: f32, height: f32) -> f32 {
    let padding = layout::page_padding(width, height);
    layout::HEADER_HEIGHT
        + padding * 2.
        + filters_height(layout::content_width(width, height))
        + FILTER_GAP
        + TABLE_HEADER
        + TABLE_FOOTER
        + TABLE_BORDER * 2.
}
fn activity_detail_height(width: f32, height: f32) -> f32 {
    // Leave room for at least three rows beneath the detail panel.
    (height - activity_chrome(width, height) - FILTER_GAP - ROW_HEIGHT * 3.).clamp(80., 240.)
}
/// How many activity rows fit under the filters, table header, and pager.
fn rows_that_fit(width: f32, height: f32, detail_open: bool) -> usize {
    let mut chrome = activity_chrome(width, height);
    if detail_open {
        chrome += FILTER_GAP + activity_detail_height(width, height);
    }
    ((height - chrome) / ROW_HEIGHT).floor().max(1.) as usize
}
fn activity_pages(current: usize, total: usize) -> Vec<Option<usize>> {
    if total <= 7 {
        return (0..total).map(Some).collect();
    }
    let mut pages = vec![Some(0)];
    let from = current.saturating_sub(2).max(1);
    let to = (current + 2).min(total - 2);
    if from > 1 {
        pages.push(None);
    }
    for page in from..=to {
        pages.push(Some(page));
    }
    if to + 1 < total - 1 {
        pages.push(None);
    }
    pages.push(Some(total - 1));
    pages
}
fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
fn reset_text(at: Option<chrono::DateTime<Utc>>) -> String {
    let Some(at) = at else {
        return "Reset not reported".into();
    };
    let remaining = at.signed_duration_since(Utc::now());
    if remaining.num_seconds() <= 0 {
        return "Window ended".into();
    }
    let minutes = remaining.num_minutes();
    if minutes < 60 {
        format!("Resets in {}m", minutes.max(1))
    } else if minutes < 1440 {
        format!("Resets in {}h {}m", minutes / 60, minutes % 60)
    } else {
        format!(
            "Resets {}",
            at.with_timezone(&Local).format("%b %-d, %H:%M")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        activity_column_widths, column_align_end, model_column_widths, reported_error, table_track,
    };
    use crate::ui::table::Column;

    #[test]
    fn status_lines_with_separators_are_not_errors() {
        let sep = token_usage::analytics::separator();
        assert!(!reported_error(&format!(
            "Account history{sep}across your machines"
        )));
        assert!(!reported_error(&format!(
            "Detected on this device{sep}no local usage yet"
        )));
        assert!(!reported_error(&format!("3 history files{sep}local")));
        assert!(reported_error("Sign in expired"));
        assert!(!reported_error(""));
    }

    #[test]
    fn header_and_rows_share_one_column_width() {
        for width in [420., 649., 650., 899., 900., 1000., 1400., 1920.] {
            for columns in [model_column_widths(width), activity_column_widths(width)] {
                assert!(columns.len() >= 3, "{width}");
                let mut names: Vec<_> = columns.iter().map(|(column, _)| *column).collect();
                let count = names.len();
                names.sort_by_key(|column| format!("{column:?}"));
                names.dedup();
                assert_eq!(names.len(), count, "duplicate column at {width}");
                let sum: f32 = columns.iter().map(|(_, column_width)| *column_width).sum();
                let track = table_track(width, columns.len());
                assert!(
                    sum + 0.01 >= track,
                    "widths {sum} fell short of track {track} at {width}: {columns:?}"
                );
                assert!(
                    columns.iter().all(|(_, column_width)| *column_width >= 48.),
                    "{width}: {columns:?}"
                );
            }
        }
        let wide = activity_column_widths(1200.);
        let model = wide
            .iter()
            .find(|(column, _)| *column == Column::Model)
            .unwrap()
            .1;
        assert!((model - (1200_f32 * 0.26).clamp(180., 320.)).abs() < 0.01);
        let project = wide
            .iter()
            .find(|(column, _)| *column == Column::Project)
            .unwrap()
            .1;
        assert!(project >= 48.);
        assert_eq!(model_column_widths(500.)[0].0, Column::Model);
        assert_eq!(activity_column_widths(500.)[0].0, Column::Time);
        assert!(
            !activity_column_widths(500.)
                .iter()
                .any(|(column, _)| *column == Column::Project)
        );
        assert!(column_align_end(Column::Cost));
        assert!(!column_align_end(Column::Time));
        assert!(!column_align_end(Column::Model));
    }
}
