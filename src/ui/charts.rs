//! Div-based charts. Every chart draws both axes: a y axis with rounded tick
//! values and gridlines (or an x value axis for horizontal bars) and a labelled
//! category axis.
use super::*;
use token_usage::analytics::Analysis;

pub const Y_AXIS_WIDTH: f32 = 48.;
pub const X_AXIS_HEIGHT: f32 = 18.;
/// Widest a single column is drawn, however few buckets there are.
pub const MAX_COLUMN: f32 = 28.;

pub struct Bar {
    /// Bottom-up `(value, color)` pairs.
    pub segments: Vec<(f64, u32)>,
    pub tooltip: String,
}

/// Rounded axis ceiling: returns `(step, ticks)` with `step * ticks >= max`.
pub fn nice_scale(max: f64) -> (f64, usize) {
    if !(max.is_finite() && max > 0.) {
        return (1., 1);
    }
    let raw = max / 4.;
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let nice = [1., 2., 2.5, 5., 10.]
        .into_iter()
        .find(|step| normalized <= *step + 1e-9)
        .unwrap_or(10.);
    let step = nice * magnitude;
    let ticks = ((max / step) - 1e-9).ceil().clamp(1., 4.) as usize;
    (step, ticks)
}
/// Compact axis numbers: 0, 250, 1.5K, 2M.
pub fn short_number(value: f64) -> String {
    let (scaled, suffix) = if value >= 1e9 {
        (value / 1e9, "B")
    } else if value >= 1e6 {
        (value / 1e6, "M")
    } else if value >= 1e3 {
        (value / 1e3, "K")
    } else {
        (value, "")
    };
    let text = if scaled >= 100. || scaled.fract().abs() < 1e-6 {
        format!("{scaled:.0}")
    } else {
        format!("{scaled:.1}").trim_end_matches(".0").to_string()
    };
    format!("{text}{suffix}")
}
pub fn short_money(value: f64, step: f64) -> String {
    if step < 0.1 {
        format!("${value:.2}")
    } else if step < 1. {
        format!("${value:.1}")
    } else {
        format!("${}", short_number(value))
    }
}
/// Evenly spaced label indices that always include the most recent bucket.
pub fn label_indices(count: usize, max_labels: usize) -> Vec<usize> {
    if count == 0 {
        return vec![];
    }
    let step = count.div_ceil(max_labels.max(1)).max(1);
    let mut indices = (0..count).rev().step_by(step).collect::<Vec<_>>();
    indices.reverse();
    indices
}

impl Dashboard {
    /// A titled chart panel with an optional control on the right.
    pub(super) fn chart_panel(
        &self,
        name: &str,
        control: Option<AnyElement>,
        height: Option<f32>,
    ) -> Div {
        let heading = title(name.to_owned(), self.theme()).into_any_element();
        self.chart_panel_with(heading, control, height)
    }
    /// A chart panel whose heading is any element, e.g. a live readout.
    pub(super) fn chart_panel_with(
        &self,
        heading: AnyElement,
        control: Option<AnyElement>,
        height: Option<f32>,
    ) -> Div {
        let theme = self.theme();
        panel(theme)
            .when_some(height, |panel, height| panel.h(px(height)))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(CHART_GAP))
            .child(
                div()
                    .h(px(CONTROL_HEIGHT))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_XS))
                    .child(heading)
                    .child(div().flex_1())
                    .children(control),
            )
    }
    pub(super) fn legend(&self, items: Vec<(u32, String)>) -> Div {
        let theme = self.theme();
        div()
            .flex_shrink_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(SPACE_M))
            .gap_y(px(SPACE_2XS))
            .children(items.into_iter().map(|(color, name)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_2XS))
                    .child(
                        div()
                            .size(px(SPACE_XS))
                            .flex_shrink_0()
                            .rounded(px(2.))
                            .bg(rgb(color)),
                    )
                    .child(
                        tabular(div())
                            .text_size(px(TYPE_CAPTION))
                            .text_color(rgb(theme.muted))
                            .child(name),
                    )
            }))
    }
    fn empty_plot(&self, text: &str) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .child(caption(text.to_owned(), self.theme()))
    }
    /// Vertical stacked columns. `height` is the whole chart: plot plus x axis.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn column_chart(
        &self,
        id: &'static str,
        bars: Vec<Bar>,
        labels: Vec<String>,
        max_labels: usize,
        money: bool,
        height: f32,
        empty: &str,
        cx: &Context<Self>,
    ) -> Div {
        let theme = self.theme();
        let maximum = bars
            .iter()
            .map(|bar| bar.segments.iter().map(|s| s.0).sum::<f64>())
            .fold(0., f64::max);
        if self.config.privacy || maximum <= 0. {
            return self
                .empty_plot(if self.config.privacy {
                    "Values hidden"
                } else {
                    empty
                })
                .h(px(height));
        }
        let (step, ticks) = nice_scale(maximum);
        let top = step * ticks as f64;
        let plot = (height - X_AXIS_HEIGHT).max(20.);
        let format = move |value: f64| {
            if money {
                short_money(value, step)
            } else {
                short_number(value)
            }
        };
        let count = bars.len();
        let gap = if count > 60 {
            1.
        } else if count > 30 {
            2.
        } else {
            4.
        };
        let hovered = self.hovered.filter(|(chart, _)| *chart == id).map(|h| h.1);
        let y_axis = div()
            .w(px(Y_AXIS_WIDTH))
            .h(px(plot))
            .flex_shrink_0()
            .relative()
            .children((0..=ticks).map(|tick| {
                let y = plot - plot * tick as f32 / ticks as f32;
                tabular(div())
                    .absolute()
                    .right(px(SPACE_XS))
                    .top(px(y - TYPE_CAPTION / 2.))
                    .h(px(TYPE_CAPTION))
                    .text_size(px(TYPE_CAPTION))
                    .line_height(px(TYPE_CAPTION))
                    .text_color(rgb(theme.faint))
                    .child(format(step * tick as f64))
            }));
        let grid = (1..=ticks).map(|tick| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(plot - plot * tick as f32 / ticks as f32))
                .h(px(1.))
                .bg(rgb(theme.grid))
        });
        let columns = div()
            .absolute()
            .inset_0()
            .px(px(gap))
            .flex()
            .items_end()
            .gap(px(gap))
            .children(bars.into_iter().enumerate().map(|(index, bar)| {
                let tooltip = bar.tooltip;
                let segments = bar
                    .segments
                    .into_iter()
                    .filter(|s| s.0 > 0.)
                    .collect::<Vec<_>>();
                let last = segments.len().saturating_sub(1);
                // Each bucket owns a full-width slot for hovering; the visible
                // column is capped so short periods do not draw wide slabs.
                div()
                    .id(SharedString::from(format!("{id}-bar-{index}")))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .justify_center()
                    .opacity(if hovered.is_some_and(|day| day != index) {
                        0.35
                    } else {
                        1.
                    })
                    .on_mouse_move(cx.listener(move |view, _, _, cx| {
                        if view.hovered != Some((id, index)) {
                            view.hovered = Some((id, index));
                            cx.notify();
                        }
                    }))
                    // Charts with a live readout pass no tooltip: the readout
                    // updates on the next frame, a tooltip only after a delay.
                    .when(!tooltip.is_empty(), |slot| {
                        slot.tooltip(move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                        })
                    })
                    .child(
                        div()
                            .w_full()
                            .max_w(px(MAX_COLUMN))
                            .h_full()
                            .flex()
                            .flex_col_reverse()
                            .gap(px(1.))
                            .children(segments.into_iter().enumerate().map(
                                move |(position, (value, color))| {
                                    div()
                                        .flex_shrink_0()
                                        .h(px((((value / top) * plot as f64) as f32 - 1.).max(1.)))
                                        .w_full()
                                        .when(position == last, |cap| cap.rounded_t(px(3.)))
                                        .bg(rgb(color))
                                },
                            )),
                    )
            }));
        let x_axis = div()
            .h(px(X_AXIS_HEIGHT))
            .ml(px(Y_AXIS_WIDTH))
            .flex_shrink_0()
            .relative()
            .children(label_indices(count, max_labels).into_iter().map(|index| {
                tabular(div())
                    .absolute()
                    .top(px(3.))
                    .left(relative((index as f32 + 0.5) / count as f32))
                    .ml(px(-32.))
                    .w(px(64.))
                    .text_center()
                    .text_size(px(TYPE_CAPTION))
                    .line_height(px(TYPE_CAPTION))
                    .text_color(rgb(theme.faint))
                    .child(labels.get(index).cloned().unwrap_or_default())
            }));
        div()
            .h(px(height))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(
                div().h(px(plot)).flex().child(y_axis).child(
                    div()
                        .id(SharedString::from(format!("{id}-plot")))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .relative()
                        .border_b_1()
                        .border_color(rgb(theme.line))
                        .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                            if !hovered && view.hovered.is_some_and(|h| h.0 == id) {
                                view.hovered = None;
                                cx.notify();
                            }
                        }))
                        .children(grid)
                        .child(columns),
                ),
            )
            .child(x_axis)
    }
    /// Horizontal bars with a value axis underneath.
    pub(super) fn bar_list(
        &self,
        rows: Vec<(String, f64, u32, String)>,
        money: bool,
        label_width: f32,
        empty: &str,
    ) -> Div {
        let theme = self.theme();
        let maximum = rows.iter().map(|row| row.1).fold(0., f64::max);
        if self.config.privacy || maximum <= 0. {
            return self.empty_plot(if self.config.privacy {
                "Values hidden"
            } else {
                empty
            });
        }
        let (step, ticks) = nice_scale(maximum);
        let top = step * ticks as f64;
        const ROW: f32 = 26.;
        const VALUE: f32 = 64.;
        let names = div()
            .w(px(label_width))
            .flex_shrink_0()
            .pr(px(SPACE_S))
            .children(rows.iter().map(|row| {
                div().h(px(ROW)).flex().items_center().child(
                    div()
                        .w_full()
                        .min_w_0()
                        .truncate()
                        .text_size(px(TYPE_CAPTION))
                        .text_color(rgb(theme.ink))
                        .child(row.0.clone()),
                )
            }));
        let plot = div()
            .flex_1()
            .min_w_0()
            .relative()
            .border_l_1()
            .border_b_1()
            .border_color(rgb(theme.line))
            .children((1..=ticks).map(|tick| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(tick as f32 / ticks as f32))
                    .w(px(1.))
                    .bg(rgb(theme.grid))
            }))
            .children(rows.iter().map(|row| {
                div().h(px(ROW)).flex().items_center().child(
                    div()
                        .h(px(12.))
                        .w(relative((row.1 / top) as f32))
                        .rounded_r(px(3.))
                        .bg(rgb(row.2)),
                )
            }));
        let values = tabular(div())
            .w(px(VALUE))
            .flex_shrink_0()
            .children(rows.iter().map(|row| {
                div()
                    .h(px(ROW))
                    .flex()
                    .items_center()
                    .justify_end()
                    .text_size(px(TYPE_CAPTION))
                    .text_color(rgb(theme.muted))
                    .child(row.3.clone())
            }));
        let axis = div()
            .h(px(X_AXIS_HEIGHT))
            .flex()
            .child(div().w(px(label_width)).flex_shrink_0())
            .child(div().flex_1().relative().children((0..=ticks).map(|tick| {
                let value = step * tick as f64;
                tabular(div())
                    .absolute()
                    .top(px(3.))
                    .left(relative(tick as f32 / ticks as f32))
                    .ml(px(-28.))
                    .w(px(56.))
                    .text_center()
                    .text_size(px(TYPE_CAPTION))
                    .line_height(px(TYPE_CAPTION))
                    .text_color(rgb(theme.faint))
                    .child(if money {
                        short_money(value, step)
                    } else {
                        short_number(value)
                    })
            })))
            .child(div().w(px(VALUE)).flex_shrink_0());
        div()
            .flex()
            .flex_col()
            .child(div().flex().child(names).child(plot).child(values))
            .child(axis)
    }
    /// Weekday-by-hour intensity grid with both axes labelled.
    pub(super) fn heatmap(&self, cells: &[[u64; 24]; 7], cell_height: f32) -> Div {
        let theme = self.theme();
        let maximum = cells.iter().flatten().copied().max().unwrap_or(0);
        if self.config.privacy || maximum == 0 {
            return self
                .empty_plot(if self.config.privacy {
                    "Values hidden"
                } else {
                    "No usage in this period"
                })
                .h(px(cell_height * 7. + 14. + X_AXIS_HEIGHT));
        }
        const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let color = theme.focus;
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(cells.iter().enumerate().map(|(day, hours)| {
                div()
                    .h(px(cell_height))
                    .flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .w(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .text_size(px(TYPE_CAPTION))
                            .line_height(px(TYPE_CAPTION))
                            .text_color(rgb(theme.faint))
                            .child(DAYS[day]),
                    )
                    .children(hours.iter().enumerate().map(|(hour, value)| {
                        let share = *value as f64 / maximum as f64;
                        let tooltip = format!(
                            "{} {hour:02}:00{}{} tokens",
                            DAYS[day],
                            analytics::separator(),
                            self.number(*value)
                        );
                        div()
                            .id(SharedString::from(format!("heat-{day}-{hour}")))
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .rounded(px(2.))
                            .bg(if *value == 0 {
                                rgb(theme.wash)
                            } else {
                                gpui::rgba((color << 8) | (0x30 + (share * 207.) as u32))
                            })
                            .tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(tooltip.clone())
                                    .build(window, cx)
                            })
                    }))
            }))
            .child(
                div()
                    .h(px(X_AXIS_HEIGHT))
                    .flex()
                    .child(div().w(px(38.)).flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .relative()
                            .children([0, 6, 12, 18, 23].map(|hour| {
                                tabular(div())
                                    .absolute()
                                    .top(px(3.))
                                    .left(relative((hour as f32 + 0.5) / 24.))
                                    .ml(px(-16.))
                                    .w(px(32.))
                                    .text_center()
                                    .text_size(px(TYPE_CAPTION))
                                    .line_height(px(TYPE_CAPTION))
                                    .text_color(rgb(theme.faint))
                                    .child(format!("{hour:02}"))
                            })),
                    ),
            )
    }
    /// Usage over the selected period, stacked by account. Hovering a column
    /// updates the header and legend on the next frame instead of opening a
    /// delayed tooltip.
    pub(super) fn usage_chart(
        &self,
        analysis: &Analysis,
        height: f32,
        width: f32,
        cx: &Context<Self>,
    ) -> Div {
        let dark = self.config.dark;
        let theme = self.theme();
        let accounts = self.visible_accounts();
        let value_of = |bucket: &token_usage::analytics::Bucket, index: usize| {
            if self.token_chart {
                bucket.tokens[index] as f64
            } else {
                bucket.cost[index]
            }
        };
        let show = |value: f64| {
            if self.config.privacy {
                analytics::hidden_value().to_string()
            } else if self.token_chart {
                self.number(value as u64)
            } else {
                self.money(value)
            }
        };
        let bars = analysis
            .buckets
            .iter()
            .map(|bucket| Bar {
                segments: Provider::ALL
                    .into_iter()
                    .map(|provider| (value_of(bucket, provider.index()), color(provider, dark)))
                    .collect(),
                tooltip: String::new(),
            })
            .collect();
        let labels = analysis
            .buckets
            .iter()
            .map(|bucket| self.period.bucket_label(bucket.start))
            .collect();
        let metric = self.segmented(
            "metric",
            vec![
                Segment::new("Tokens", self.token_chart, UiAction::Metric(true)),
                Segment::new("Cost", !self.token_chart, UiAction::Metric(false)),
            ],
            56.,
            cx,
        );
        let hovered = self
            .hovered
            .filter(|(chart, _)| *chart == "usage")
            .and_then(|(_, index)| analysis.buckets.get(index));
        // Per-account values for the hovered bucket, or the whole period.
        let per_account = |index: usize| match hovered {
            Some(bucket) => value_of(bucket, index),
            None => analysis.buckets.iter().map(|b| value_of(b, index)).sum(),
        };
        let readout = show(Provider::ALL.iter().map(|p| per_account(p.index())).sum());
        let context = match hovered {
            Some(bucket) => format!(
                "{} - {}",
                self.period.bucket_label(bucket.start),
                self.period.bucket_label(bucket.end)
            ),
            None => self.period.label().to_lowercase(),
        };
        let header = div()
            .flex()
            .items_baseline()
            .gap(px(SPACE_XS))
            .child(
                tabular(div())
                    .font_family(display_font())
                    .text_size(px(TYPE_DISPLAY))
                    .line_height(relative(1.1))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme.ink))
                    .child(readout),
            )
            .child(caption(context, theme));
        let legend = height >= 190.;
        let plot = chart_block_height(height, legend);
        // Accounts with usage anywhere in the period keep their legend slot
        // while hovering, so the legend does not jump between columns.
        let listed = accounts
            .iter()
            .filter(|s| {
                analysis
                    .buckets
                    .iter()
                    .any(|b| value_of(b, s.provider.index()) > 0.)
            })
            .map(|s| {
                (
                    color(s.provider, dark),
                    format!(
                        "{}  {}",
                        s.provider.name(),
                        show(per_account(s.provider.index()))
                    ),
                )
            })
            .collect();
        self.chart_panel_with(
            header.into_any_element(),
            Some(metric.into_any_element()),
            Some(height),
        )
        .child(self.column_chart(
            "usage",
            bars,
            labels,
            ((width - Y_AXIS_WIDTH) / 84.).floor().max(2.) as usize,
            !self.token_chart,
            plot.max(40.),
            if self.token_chart {
                "No token usage"
            } else {
                "No priced usage"
            },
            cx,
        ))
        .when(legend, |panel| panel.child(self.legend(listed)))
    }
}

#[cfg(test)]
mod tests {
    use super::{label_indices, nice_scale, short_money, short_number};
    #[test]
    fn scale_rounds_up_to_a_readable_ceiling() {
        assert_eq!(nice_scale(0.), (1., 1));
        let (step, ticks) = nice_scale(4_270_000.);
        assert!(step * ticks as f64 >= 4_270_000.);
        assert_eq!(step, 2_000_000.);
        assert_eq!(ticks, 3);
        let (step, ticks) = nice_scale(0.31);
        assert!((step - 0.1).abs() < 1e-9 && ticks == 4);
        for max in [1., 7., 99., 1234., 0.003] {
            let (step, ticks) = nice_scale(max);
            assert!(step * ticks as f64 >= max && ticks <= 4, "{max}");
        }
    }
    #[test]
    fn axis_text_is_short() {
        assert_eq!(short_number(0.), "0");
        assert_eq!(short_number(2_500.), "2.5K");
        assert_eq!(short_number(2_000_000.), "2M");
        assert_eq!(short_money(0.25, 0.05), "$0.25");
        assert_eq!(short_money(20., 5.), "$20");
    }
    #[test]
    fn labels_include_the_latest_bucket() {
        assert_eq!(label_indices(30, 6), vec![4, 9, 14, 19, 24, 29]);
        assert_eq!(label_indices(3, 10), vec![0, 1, 2]);
        assert!(label_indices(0, 4).is_empty());
    }
}
