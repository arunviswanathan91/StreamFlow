//! Screens for modules that exist in the original app but are not built into the Rust front end yet.
//! They say plainly what the module does and how far the Rust port is, instead of pretending.

use super::{theme, Module};
use eframe::egui::{self, RichText};

/// `(what the original module does, status of the Rust port)`.
pub fn info(m: Module) -> (&'static str, &'static str) {
    match m {
        Module::DimReduction => (
            "Pooled UMAP / t-SNE across samples with equal-N downsampling, marker selection, debris and live/dead clean-up, and gating on the embedding (marker rules, cluster approval).",
            "Engine DONE and verified against openTSNE and umap-learn (neighbourhood preservation, graph edge for edge). Screen NOT built yet.",
        ),
        Module::Clustering => (
            "FlowSOM, PhenoGraph, k-means and others on marker values; cluster heat map; clusters named by their markers and approved as populations.",
            "FlowSOM and PhenoGraph engines DONE (agree with the Python packages); cluster naming and approval DONE in the core. Screen NOT built yet.",
        ),
        Module::Spectral => (
            "Spectral unmixing: build reference signatures from single-stain controls, unmix with OLS / weighted least squares, per-cell autofluorescence, signature quality control.",
            "Engine DONE (matches the Python module and published method). Screen NOT built yet.",
        ),
        Module::CellCycle => ("Fit G0/G1, S and G2/M phases to a DNA-content histogram (Dean–Jett–Fox style) and report phase percentages.", "Not ported yet (the Python engine has it)."),
        Module::Proliferation => ("Dye-dilution proliferation modelling: generation peaks, division index, proliferation index.", "Not ported yet (the Python engine has it)."),
        Module::Apoptosis => ("Annexin V / viability quadrant analysis into live, early and late apoptotic and necrotic fractions per sample.", "Not ported yet. Quadrant gates already exist in the Workstation."),
        Module::StatsComparison => ("Compare a statistic between groups of samples with the appropriate test and effect size.", "Not ported yet."),
        Module::Kinetic => ("Signal versus time (calcium flux and similar), smoothed kinetics with gating on time.", "Not ported yet."),
        Module::Classifier => ("Train a classifier on labelled populations and apply it to new samples.", "Not ported yet."),
        Module::CrossSample => ("Compare populations and marker expression across all loaded samples side by side.", "Not ported yet. The Workstation Overview already shows any population across samples."),
        Module::Longitudinal => ("Track populations across time points per donor.", "Not ported yet."),
        Module::Scatter3d => ("Interactive three-parameter scatter.", "Not ported yet."),
        Module::Plugins => ("Run optional analysis plugins (R-based tools such as PeacoQC and FlowMagic).", "Not ported yet. By decision, R is plugins-only; the core runs without it."),
        _ => ("", ""),
    }
}

pub fn placeholder(ui: &mut egui::Ui, m: Module) {
    let (what, status) = info(m);
    egui::Frame::none().inner_margin(egui::Margin::same(28.0)).show(ui, |ui| {
        ui.label(RichText::new(format!("{}  {}", m.icon(), m.label())).strong().size(22.0));
        ui.add_space(10.0);
        egui::Frame::none().fill(theme::PANEL_2).rounding(10.0).inner_margin(egui::Margin::same(16.0)).show(ui, |ui| {
            ui.set_width(ui.available_width().min(720.0));
            theme::section(ui, "In the original app");
            ui.label(what);
            ui.add_space(10.0);
            theme::section(ui, "State of the Rust port");
            ui.label(RichText::new(status).color(theme::WARN));
        });
    });
}
