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
        PlotKind::Pseudocolor | PlotKind::Dot => {}
    }
    ui.checkbox(&mut s.grid, "Show grid");
    if kind != PlotKind::Histogram {
        ui.checkbox(&mut s.adjunct, "Adjunct histograms").on_hover_text("Histograms of the X and Y parameters along the edges of the plot (shown in exports and layouts)");
    }
    if ui.small_button("Reset these options").clicked() {
        *s = PlotStyle::default();
    }
    *s = s.clone().clamped();
    *s != before
}
