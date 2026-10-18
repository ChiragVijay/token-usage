use super::*;
use charts::{Bar, Y_AXIS_WIDTH};
use token_usage::analytics::{ProjectRow, Totals};
use views::{COLUMN_GAP, ROW_HEIGHT, TABLE_HEADER, TABLE_PAD};

impl Dashboard {
    pub(super) fn insights_view(&self, width: f32, height: f32, cx: &Context<Self>) -> Div {
        let dark = self.config.dark;
        let content = layout::content_width(width, height).min(1200.);
        let two = content >= 900.;
        let half = if two { (content - 12.) / 2. } else { content };
        let analysis =
            self.analysis_or_empty(self.derived.scoped.value(), self.period, self.provider);
        let insights = self.derived.insights.value().unwrap_or_else(|| {
            std::sync::Arc::new(analytics::insights(
                &Snapshot::default(),
                &self.config,
                self.period,
                self.provider,
                self.now,
            ))
        });
        let projects = self.derived.projects.value().unwrap_or_default();

        let colors = token_colors(dark);
        const KINDS: [&str; 4] = ["Input", "Output", "Cache read", "Cache write"];
        let kinds = |tokens: &token_usage::model::Tokens| {
            [
                tokens.input,
                tokens.output,
                tokens.cache_read,
                tokens.cache_write,
            ]
        };
        let bars = insights
            .mix
            .iter()
            .map(|bucket| Bar {
                segments: kinds(&bucket.tokens)
                    .into_iter()
                    .zip(colors)
                    .map(|(value, color)| (value as f64, color))
                    .collect(),
                tooltip: String::new(),
            })
            .collect();
        // Hovering a column shows its breakdown in the legend right away.
        let hovered = self
            .hovered
            .filter(|(chart, _)| *chart == "mix")
            .and_then(|(_, index)| insights.mix.get(index));
        let shown = match hovered {
            Some(bucket) => kinds(&bucket.tokens),
            None => insights.mix.iter().fold([0; 4], |mut sum, bucket| {
                for (total, value) in sum.iter_mut().zip(kinds(&bucket.tokens)) {
                    *total += value;
                }
                sum
            }),
        };
        let mix_heading = div()
            .flex()
            .items_baseline()
            .gap(px(SPACE_XS))
            .child(title("Token mix", self.theme()))
            .when_some(hovered, |row, bucket| {
                row.child(caption(
                    format!(
                        "{} - {}",
                        self.period.bucket_label(bucket.start),
                        self.period.bucket_label(bucket.end)
                    ),
                    self.theme(),
                ))
            });
        let labels = insights
            .mix
            .iter()
            .map(|bucket| self.period.bucket_label(bucket.start))
            .collect();
        let mix = self
            .chart_panel_with(mix_heading.into_any_element(), None, Some(300.))
            .child(
                self.column_chart(
                    "mix",
                    bars,
                    labels,
                    ((content - SPACE_M * 2. - Y_AXIS_WIDTH) / 84.)
                        .floor()
                        .max(2.) as usize,
                    false,
                    chart_block_height(300., true),
                    "No token usage",
                    cx,
                ),
            )
            .child(
                self.legend(
                    KINDS
                        .iter()
                        .zip(colors)
                        .zip(shown)
                        .map(|((kind, color), value)| {
                            (color, format!("{kind}  {}", self.number(value)))
                        })
                        .collect(),
                ),
            );

        let accounts = self.visible_accounts();
        let account_rows = |cost: bool| {
            let mut rows = accounts
                .iter()
                .filter(|status| self.provider.is_none_or(|p| p == status.provider))
                .filter_map(|status| {
                    let totals = &analysis.providers[status.provider.index()];
                    let value = if cost {
                        totals.spend()?
                    } else {
                        totals.tokens.total() as f64
                    };
                    Some((
                        status.provider.name().to_owned(),
                        value,
                        color(status.provider, dark),
                        if cost {
                            self.spend(totals)
                        } else {
                            self.number(totals.tokens.total())
                        },
                    ))
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| b.1.total_cmp(&a.1));
            rows
        };
        let by_cost = self
            .chart_panel("Cost by account", None, None)
            .flex_1()
            .min_w_0()
            .child(self.bar_list(account_rows(true), true, 96., "No priced usage"));
        let by_tokens = self
            .chart_panel("Tokens by account", None, None)
            .flex_1()
            .min_w_0()
            .child(self.bar_list(account_rows(false), false, 96., "No token usage"));

        let mut models = analysis.models.clone();
        models.truncate(8);
        let label_width = (half * 0.34).clamp(110., 200.);
        let top_models = self
            .chart_panel("Top models", None, None)
            .flex_1()
            .min_w_0()
            .child(
                self.bar_list(
                    models
                        .iter()
                        .map(|row| {
                            (
                                row.model.clone(),
                                row.totals.tokens.total() as f64,
                                color(row.provider, dark),
                                self.number(row.totals.tokens.total()),
                            )
                        })
                        .collect(),
                    false,
                    label_width,
                    "No model usage",
                ),
            );
        let hours = self
            .chart_panel("Tokens by hour", None, None)
            .flex_1()
            .min_w_0()
            .child(self.heatmap(&insights.heatmap, 22.));

        let pair = |a: Div, b: Div| {
            if two {
                div().flex().gap(px(SPACE_S)).child(a).child(b)
            } else {
                div().flex().flex_col().gap(px(SPACE_S)).child(a).child(b)
            }
        };
        div()
            .w_full()
            .max_w(px(1200.))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(SPACE_S))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(self.account_select("insights", cx))
                    .child(div().flex_1()),
            )
            .child(self.ratio_tiles(&analysis.totals, content))
            .child(mix)
            .child(pair(by_cost, by_tokens))
            .child(pair(top_models, hours))
            .child(self.projects_table(&projects, content, cx))
    }
    fn ratio_tiles(&self, totals: &Totals, width: f32) -> Div {
        let tokens = totals.tokens.total();
        let share = |part: u64| {
            if self.config.privacy {
                analytics::hidden_value().to_string()
            } else if tokens == 0 {
                analytics::empty_value().into()
            } else {
                format!("{:.0}%", part as f64 / tokens as f64 * 100.)
            }
        };
        let per = |count: f64| match totals.spend() {
            Some(cost) if count > 0. => self.money(cost / count),
            _ => analytics::empty_value().into(),
        };
        let tiles = [
            ("Cache read", share(totals.tokens.cache_read)),
            ("Output", share(totals.tokens.output)),
            ("Per 1M tokens", per(tokens as f64 / 1e6)),
            ("Per session", per(totals.sessions as f64)),
        ];
        let columns = if width >= 640. { 4 } else { 2 };
        div()
            .flex()
            .flex_col()
            .gap(px(SPACE_S))
            .children(tiles.chunks(columns).map(|row| {
                div().flex().gap(px(SPACE_S)).children(
                    row.iter()
                        .map(|(name, value)| self.stat_tile(name, value.clone(), None, false)),
                )
            }))
    }
    /// `rows` arrive sorted from the background; only the first 100 are built.
    fn projects_table(&self, rows: &[ProjectRow], width: f32, cx: &Context<Self>) -> Div {
        let theme = self.theme();
        let sort = self.project_sort;
        let narrow = width < 650.;
        let numbers: &[(Column, f32)] = if narrow {
            &[(Column::Tokens, 72.), (Column::Cost, 76.)]
        } else {
            &[
                (Column::Sessions, 76.),
                (Column::Events, 76.),
                (Column::Tokens, 88.),
                (Column::Cost, 96.),
            ]
        };
        let header = div()
            .h(px(TABLE_HEADER))
            .px(px(TABLE_PAD))
            .flex()
            .items_center()
            .gap(px(COLUMN_GAP))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(theme.line))
            .bg(rgb(theme.bg))
            .child(
                self.sort_header(Table::Projects, Column::Project, None, false, false, cx)
                    .flex_1()
                    .min_w_0(),
            )
            .children(numbers.iter().map(|(column, width)| {
                self.sort_header(Table::Projects, *column, Some(*width), true, false, cx)
            }));
        let body = div().children(rows.iter().take(100).enumerate().map(|(index, row)| {
            div()
                .h(px(ROW_HEIGHT))
                .px(px(TABLE_PAD))
                .flex()
                .items_center()
                .gap(px(COLUMN_GAP))
                .when(index > 0, |line| {
                    line.border_t_1().border_color(rgb(theme.grid))
                })
                .child(
                    self.table_cell(
                        if self.config.privacy {
                            "Hidden"
                        } else if row.project.is_empty() {
                            "Not reported"
                        } else {
                            &row.project
                        },
                        None,
                        false,
                        row.project.is_empty() || self.config.privacy,
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .children(numbers.iter().map(|(column, width)| {
                    let text = match column {
                        Column::Sessions => self.number(row.totals.sessions as u64),
                        Column::Events => self.number(row.totals.events as u64),
                        Column::Tokens => self.number(row.totals.tokens.total()),
                        _ => self.cost_text(&row.totals),
                    };
                    self.table_cell(&text, Some(*width), true, false)
                }))
        }));
        panel(theme)
            .p(px(0.))
            .overflow_hidden()
            .child(
                div()
                    .h(px(SPACE_XL + SPACE_M))
                    .px(px(TABLE_PAD))
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(title("Projects", theme))
                    .child(caption(format!("{}", rows.len()), theme))
                    .child(div().flex_1())
                    .when(!sort.is_default(), |row| {
                        row.child(self.text_action(
                            "projects-reset-sort",
                            "Reset sort",
                            Some("reset"),
                            UiAction::ResetSort(Table::Projects),
                            cx,
                        ))
                    }),
            )
            .child(header)
            .when(rows.is_empty(), |panel| {
                panel.child(
                    div()
                        .py(px(SPACE_L))
                        .flex()
                        .justify_center()
                        .child(caption("No usage in this period", theme)),
                )
            })
            .child(body)
    }
    /// Reported cost, `~` for an API-rate estimate, `-` when unpriced.
    pub(super) fn cost_text(&self, totals: &Totals) -> String {
        match totals.spend() {
            None => analytics::empty_value().into(),
            Some(cost) if totals.reported_rows == 0 && !self.config.privacy => {
                format!("~{}", self.money(cost))
            }
            Some(cost) => self.money(cost),
        }
    }
}
