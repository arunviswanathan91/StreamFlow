//! The plot-style controls shared by the graph window, the layout editor and the grid layout: the histogram,
//! density and contour options (FlowJo's labels, plus the options the previous app had).

use super::theme;
use crate::density::{HistStyle, HistY, PlotKind, PlotStyle};
use eframe::egui::{self, RichText};

/// Show the options that apply to `kind`. Returns `true` when anything changed.
pub(super) fn plot_style_controls(ui: &mut egui::Ui, kind: PlotKind, s: &mut PlotStyle, salt: &str) -> bool {
    let before = s.clone();
    match kind {
        PlotKind::Histogram => {
            ui.label(RichText::new("Y axis").size(11.5).color(theme::MUTED));
            egui::ComboBox::from_id_salt((salt, "hist_y")).width(190.0).selected_text(s.hist_y.label()).show_ui(ui, |ui| {
                for y in HistY::ALL {
                    ui.selectable_value(&mut s.hist_y, y, y.label());
                }
            });
            ui.horizontal(|ui| {
                for st in HistStyle::ALL {
                    ui.selectable_value(&mut s.hist_style, st, st.label());
                }
            });
            let smoothable = s.hist_style != HistStyle::Bars && s.hist_y != HistY::Cdf;
            ui.add_enabled_ui(smoothable, |ui| {
                ui.checkbox(&mut s.hist_smooth, "Smoothing").on_hover_text("Gaussian averaging to reduce noise (not applied to bars or a cumulative curve)");
                if s.hist_smooth {
                    ui.add(egui::Slider::new(&mut s.hist_bandwidth, 0.0..=1.0).text("Bandwidth"));
                }
            });
            if !smoothable {
                theme::caption(ui, "Bars and the cumulative curve show the raw counts.");
            }
            ui.horizontal(|ui| {
                ui.label("Y max");
                ui.add(egui::DragValue::new(&mut s.hist_ymax).speed(1.0).range(0.0..=1.0e9).custom_formatter(|v, _| if v == 0.0 { "auto".to_string() } else { format!("{v:.0}") })).on_hover_text("Top of the Y axis in the chosen unit (auto = fit)");
            });
            ui.horizontal(|ui| {
                ui.label("Fill");
                ui.color_edit_button_srgb(&mut s.hist_fill);
                ui.add(egui::Slider::new(&mut s.hist_fill_alpha, 0..=255).text("opacity"));
            });
            ui.horizontal(|ui| {
                ui.label("Line");
                ui.color_edit_button_srgb(&mut s.hist_line);
            });
        }
        PlotKind::Contour => {
            ui.add(egui::Slider::new(&mut s.contour_levels, 3..=30).text("Contour levels")).on_hover_text("Each line encloses an equal further share of the events");
            ui.checkbox(&mut s.contour_smooth, "Smooth contours");
            if s.contour_smooth {
                ui.add(egui::Slider::new(&mut s.contour_strength, 1..=8).text("Smoothness"));
            }
            ui.horizontal(|ui| {
                ui.label("Line colour");
                ui.color_edit_button_srgb(&mut s.contour_color);
            });
            ui.checkbox(&mut s.outliers, "Show outliers").on_hover_text("Draw single events outside the outermost contour");
            if s.outliers {
                ui.add(egui::Slider::new(&mut s.outlier_size, 0..=6).text("Outlier size"));
            }
        }
        PlotKind::Density | PlotKind::Zebra => {
            ui.checkbox(&mut s.smooth, "Smooth density");
            if s.smooth {
                ui.add(egui::Slider::new(&mut s.smooth_strength, 1..=8).text("Smoothness"));
            }
        }
        PlotKind::Pseudocolor => {
            ui.add(egui::Slider::new(&mut s.pseudo_levels, 4..=256).text("Levels")).on_hover_text("Number of colour bands; 256 is continuous");
            ui.checkbox(&mut s.pseudo_smooth, "Smoothing").on_hover_text("Gaussian averaging to reduce noise");
            if s.pseudo_smooth {
                ui.add(egui::Slider::new(&mut s.smooth_strength, 1..=8).text("Smoothness"));
            }
            ui.checkbox(&mut s.pseudo_outliers, "Show outliers").on_hover_text("Off hides the sparsest 1% of events");
        }
        PlotKind::Dot => {
            ui.checkbox(&mut s.large_dots, "Use large dots");
            ui.horizontal(|ui| {
                ui.label("Max dots");
                ui.add(egui::DragValue::new(&mut s.num_dots).speed(100.0).range(0..=5_000_000).custom_formatter(|v, _| if v == 0.0 { "all".to_string() } else { format!("{v:.0}") }));
            });
        }
    }
    ui.checkbox(&mut s.grid, "Show grid");
    ui.checkbox(&mut s.sci, "Use scientific notation");
    if kind != PlotKind::Histogram {
        ui.checkbox(&mut s.adjunct, "Adjunct histograms").on_hover_text("Histograms of the X and Y parameters along the edges of the plot (shown in exports and layouts)");
    }
    if ui.small_button("Reset these options").clicked() {
        *s = PlotStyle::default();
    }
    *s = s.clone().clamped();
    *s != before
}
