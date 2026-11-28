//! Cell Cycle, Proliferation and Apoptosis screens. Each one fits a population of one sample on one (or two)
//! channels with the ports in `sf_core::analysis`, shows the fit, and reports the numbers.

use super::{theme, App};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke};
use sf_core::analysis::{apoptosis, cell_cycle, proliferation, ApoptosisResult, CycleModel, CycleOptions, CycleResult, ProliferationResult};
use sf_gating::GateTree;

const G1: Color32 = Color32::from_rgb(0x4C, 0xAF, 0x50);
const S_PHASE: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
const G2: Color32 = Color32::from_rgb(0xE0, 0x55, 0x55);
const FIT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub(super) const GEN: [Color32; 8] = [
    Color32::from_rgb(0x1F, 0x77, 0xB4),
    Color32::from_rgb(0xFF, 0x7F, 0x0E),
    Color32::from_rgb(0x2C, 0xA0, 0x2C),
    Color32::from_rgb(0xD6, 0x27, 0x28),
    Color32::from_rgb(0x94, 0x67, 0xBD),
    Color32::from_rgb(0x8C, 0x56, 0x4B),
    Color32::from_rgb(0xE3, 0x77, 0xC2),
    Color32::from_rgb(0xBC, 0xBD, 0x22),
];

/// Which analysis a screen runs; each keeps its own channel choices.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Cycle,
    Prolif,
    Apoptosis,
}

pub struct State {
    pub sample: usize,
    pub restrict: Vec<String>,
    pub cycle_channel: String,
    pub prolif_channel: String,
    pub annexin_channel: String,
    pub pi_channel: String,
    pub dj_fox: bool,
    pub use_hint: bool,
    pub mu_hint: f64,
    pub generations: usize,
    pub cofactor: f64,
    pub cycle: Option<(String, CycleResult)>,
    pub prolif: Option<(String, ProliferationResult)>,
    pub apop: Option<(String, ApoptosisResult, Vec<(f64, f64)>)>,
}

impl Default for State {
    fn default() -> Self {
        State {
            sample: 0,
            restrict: vec![],
            cycle_channel: String::new(),
            prolif_channel: String::new(),
            annexin_channel: String::new(),
            pi_channel: String::new(),
            dj_fox: true,
            use_hint: false,
            mu_hint: 0.0,
            generations: 8,
            cofactor: 150.0,
            cycle: None,
            prolif: None,
            apop: None,
        }
    }
}

/// First channel whose name contains one of the hints (case-insensitive), preferring area (`-A`) channels.
pub fn guess_channel(channels: &[String], hints: &[&str]) -> Option<String> {
    let lower: Vec<String> = channels.iter().map(|c| c.to_lowercase()).collect();
    for h in hints {
        let h = h.to_lowercase();
        let hits: Vec<usize> = (0..channels.len()).filter(|&i| lower[i].contains(&h)).collect();
        if let Some(&i) = hits.iter().find(|&&i| lower[i].ends_with("-a")).or(hits.first()) {
            return Some(channels[i].clone());
        }
    }
    None
}

impl App {
    fn ana_sync(&mut self) {
        let n = self.session.samples().len();
        if self.ana.sample >= n {
            self.ana.sample = 0;
            self.ana.restrict.clear();
        }
        if n == 0 {
            return;
        }
        let chans = self.session.sample(self.ana.sample).channels.clone();
        let fluor: Vec<String> = chans.iter().filter(|c| !c.starts_with("FSC") && !c.starts_with("SSC") && c.as_str() != "Time" && !c.ends_with("-W") && !c.ends_with("-H")).cloned().collect();
        let fix = |cur: &mut String, hints: &[&str], fallback: Option<&String>| {
            if cur.is_empty() || !chans.contains(cur) {
                *cur = guess_channel(&chans, hints).or_else(|| fallback.cloned()).unwrap_or_default();
            }
        };
        fix(&mut self.ana.cycle_channel, &["PI", "DAPI", "7-AAD", "7AAD", "Hoechst", "DNA", "FxCycle", "DRAQ"], fluor.first());
        fix(&mut self.ana.prolif_channel, &["CFSE", "CTV", "CellTrace", "Violet", "eFluor 670", "FITC"], fluor.first());
        fix(&mut self.ana.annexin_channel, &["Annexin", "FITC", "FL1"], fluor.first());
        fix(&mut self.ana.pi_channel, &["PI", "7-AAD", "7AAD", "FL2", "PE"], fluor.get(1).or(fluor.first()));
    }

    /// Raw values of `channel` for the chosen population of the chosen sample.
    fn ana_values(&mut self, channel: &str) -> Result<Vec<f64>, String> {
        let si = self.ana.sample;
        let path: Vec<String> = self.ana.restrict.clone();
        let node = if path.is_empty() {
            GateTree::ROOT
        } else {
            let refs: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
            self.session.tree(si).find(&refs).ok_or_else(|| format!("population '{}' does not exist in this sample", path.join(" ▸ ")))?
        };
        self.session.population_values(si, node, channel)
    }

    fn ana_label(&self) -> String {
        let name = self.session.samples()[self.ana.sample].name.clone();
        if self.ana.restrict.is_empty() {
            name
        } else {
            format!("{name} · {}", self.ana.restrict.join(" ▸ "))
        }
    }

    pub fn run_cell_cycle(&mut self) {
        self.ana_sync();
        let ch = self.ana.cycle_channel.clone();
        let result = self.ana_values(&ch).and_then(|v| {
            let opt = CycleOptions { mu_g1_hint: if self.ana.use_hint && self.ana.mu_hint > 0.0 { Some(self.ana.mu_hint) } else { None }, ..Default::default() };
            cell_cycle(&v, if self.ana.dj_fox { CycleModel::DeanJettFox } else { CycleModel::Watson }, &opt)
        });
        match result {
            Ok(r) => {
                self.status = format!("Cell cycle: G0/G1 {:.1}%, S {:.1}%, G2/M {:.1}% (R² {:.3}).", r.g1_pct, r.s_pct, r.g2_pct, r.r2);
                self.ana.cycle = Some((self.ana_label(), r));
            }
            Err(e) => {
                self.status = e;
                self.ana.cycle = None;
            }
        }
    }

    pub fn run_proliferation(&mut self) {
        self.ana_sync();
        let ch = self.ana.prolif_channel.clone();
        let g = self.ana.generations;
        match self.ana_values(&ch).and_then(|v| proliferation(&v, g, None)) {
            Ok(r) => {
                self.status = format!("Proliferation: {} generations, PI {:.2}, DI {:.2}, {:.1}% divided.", r.generations.len(), r.pi, r.di, r.pct_divided);
                self.ana.prolif = Some((self.ana_label(), r));
            }
            Err(e) => {
                self.status = e;
                self.ana.prolif = None;
            }
        }
    }

    pub fn run_apoptosis(&mut self) {
        self.ana_sync();
        let (a_ch, p_ch) = (self.ana.annexin_channel.clone(), self.ana.pi_channel.clone());
        if a_ch == p_ch {
            self.status = "Choose two different channels for Annexin V and the viability dye.".into();
            return;
        }
        let res = self.ana_values(&a_ch).and_then(|a| self.ana_values(&p_ch).map(|p| (a, p))).and_then(|(a, p)| {
            let r = apoptosis(&a, &p, self.ana.cofactor)?;
            // an even stride through the finite events, at most 5000 points, for the picture
            let pts: Vec<(f64, f64)> = a.iter().zip(&p).filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| (*x, *y)).collect();
            let step = (pts.len() / 5000).max(1);
            Ok((r, pts.into_iter().step_by(step).collect::<Vec<_>>()))
        });
        match res {
            Ok((r, pts)) => {
                let q = r.quadrants;
                self.status = format!("Apoptosis: live {:.1}%, early {:.1}%, late {:.1}%, necrotic {:.1}%.", q.live, q.early, q.late, q.necrotic);
                self.ana.apop = Some((self.ana_label(), r, pts));
            }
            Err(e) => {
                self.status = e;
                self.ana.apop = None;
            }
        }
    }

    /// The population the analysis runs on, in sample `si` (`None` when that sample has no such population).
    fn ana_node(&self, si: usize) -> Option<sf_gating::NodeId> {
        if self.ana.restrict.is_empty() {
            return Some(GateTree::ROOT);
        }
        let refs: Vec<&str> = self.ana.restrict.iter().map(|s| s.as_str()).collect();
        self.session.tree(si).find(&refs)
    }

    /// The names of the gates an analysis makes, in the order they are shown.
    pub fn ana_gate_names(kind: Kind) -> &'static [&'static str] {
        match kind {
            Kind::Apoptosis => &["Live", "Early apoptotic", "Late apoptotic", "Necrotic"],
            Kind::Cycle => &["G0/G1", "S", "G2/M"],
            Kind::Prolif => &[],
        }
    }

    /// Turn the last result into ORDINARY gates under the analysed population and give them to All Samples: a
    /// quadrant gate at the two thresholds (apoptosis) or three interval gates at the phase boundaries (cell
    /// cycle). From then on they are gates like any other: dragged on the plot, compared across samples, counted
    /// in Statistics, exported. Running again moves the existing gates instead of adding new ones.
    pub fn ana_make_gates(&mut self, kind: Kind) -> Result<String, String> {
        use crate::density::PlotKind;
        use crate::drawing::{Shape as Drawn, Tool};
        use sf_core::scale::ScaleSpec;
        let si = self.ana.sample;
        let node = self.ana_node(si).ok_or("the chosen population does not exist in this sample")?;
        let id = self.session.open_population(si, node)?;
        let existing = |app: &Self, name: &str| app.session.tree(si).children(node).ok().and_then(|c| c.iter().copied().find(|&k| app.session.tree(si).name(k).ok() == Some(name)));
        let mut made: Vec<sf_gating::NodeId> = Vec::new();
        let updated;
        match kind {
            Kind::Apoptosis => {
                let (ta, tp) = self.ana.apop.as_ref().map(|(_, r, _)| (r.annexin_threshold, r.pi_threshold)).ok_or("Run the analysis first")?;
                let scale = if self.ana.cofactor > 0.0 { ScaleSpec::Arcsinh { cofactor: self.ana.cofactor } } else { ScaleSpec::Identity };
                let t = scale.build()?;
                let (cx, cy) = (t.forward(ta), t.forward(tp));
                let (xc, yc) = (self.ana.annexin_channel.clone(), self.ana.pi_channel.clone());
                {
                    let w = self.session.window_mut(id).ok_or("no window")?;
                    (w.x_chan, w.y_chan, w.x_scale, w.y_scale, w.kind, w.tool) = (xc, yc, scale.clone(), scale, PlotKind::Pseudocolor, Tool::Pointer);
                    w.drawing.cancel();
                }
                match existing(self, "Live") {
                    Some(live) if !self.session.quadrant_set(si, live).is_empty() => {
                        self.session.move_quadrants(si, live, cx, cy)?;
                        made = self.session.quadrant_set(si, live);
                        updated = true;
                    }
                    Some(_) => return Err("A population called 'Live' already exists here and is not a quadrant gate; rename or delete it first".into()),
                    None => {
                        // the quadrant tool's order: x+y+, x-y+, x-y-, x+y-
                        let ids = self.session.commit_quadrants(id, cx, cy)?;
                        for (n, name) in ids.iter().zip(["Late apoptotic", "Necrotic", "Live", "Early apoptotic"]) {
                            self.session.rename(si, *n, name)?;
                        }
                        made = ids;
                        updated = false;
                    }
                }
            }
            Kind::Cycle => {
                let b = self.ana.cycle.as_ref().map(|(_, r)| r.boundaries).ok_or("Run the analysis first")?;
                let ch = self.ana.cycle_channel.clone();
                {
                    let w = self.session.window_mut(id).ok_or("no window")?;
                    (w.x_chan, w.x_scale, w.kind, w.tool) = (ch.clone(), ScaleSpec::Identity, PlotKind::Histogram, Tool::Pointer);
                    w.drawing.cancel();
                }
                let mut any_old = false;
                for (name, lo, hi) in [("G0/G1", b[0], b[1]), ("S", b[1], b[2]), ("G2/M", b[2], b[3])] {
                    if !(hi > lo) {
                        return Err(format!("the fit gave no room for {name}; the phase boundaries are {b:?}"));
                    }
                    match existing(self, name) {
                        Some(k) => {
                            let mut g = self.session.gate_of(si, k).filter(|g| matches!(g.kind, sf_gating::GateKind::Interval) && g.x_chan == ch).ok_or_else(|| format!("A population called '{name}' already exists here and is not an interval on {ch}; rename or delete it first"))?;
                            g.xs = vec![lo, hi];
                            self.session.edit_gate(si, k, g)?;
                            made.push(k);
                            any_old = true;
                        }
                        None => made.push(self.session.commit_shape(id, &Drawn::Interval { x0: lo, x1: hi }, name)?),
                    }
                }
                updated = any_old;
            }
            Kind::Prolif => return Err("Generation gates are not made yet".into()),
        }
        // every sample gets the same gates (a sample without the parent population is named in the Analysis Log)
        for &n in &made {
            let _ = self.session.apply_to_group(si, n, crate::session::ALL_SAMPLES);
        }
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.refit = true;
        }
        let n = self.session.samples().len();
        Ok(format!("{} {} gate(s) on {n} sample(s): {}. They are ordinary gates now: drag them on the plot (Workstation), or compare the samples in the table.", if updated { "Updated" } else { "Made" }, made.len(), Self::ana_gate_names(kind).join(", ")))
    }

    /// Percent of the analysed population inside each of the analysis' gates, for every sample (`None` where a
    /// sample lacks the population or the gate).
    pub fn ana_gate_table(&mut self, kind: Kind) -> Vec<(usize, Vec<Option<f64>>)> {
        let names = Self::ana_gate_names(kind);
        (0..self.session.samples().len())
            .map(|i| {
                let Some(parent) = self.ana_node(i) else { return (i, vec![None; names.len()]) };
                let total = self.session.count(i, parent).max(1) as f64;
                let row = names
                    .iter()
                    .map(|name| {
                        let k = self.session.tree(i).children(parent).ok()?.iter().copied().find(|&k| self.session.tree(i).name(k).ok() == Some(*name))?;
                        Some(100.0 * self.session.count(i, k) as f64 / total)
                    })
                    .collect();
                (i, row)
            })
            .collect()
    }

    /// Under a result: make the gates, and the table of every sample read from those gates.
    fn ana_gates_ui(&mut self, ui: &mut egui::Ui, kind: Kind) {
        ui.add_space(10.0);
        ui.separator();
        theme::section(ui, "Gates on all samples");
        theme::caption(ui, match kind {
            Kind::Apoptosis => "Puts a quadrant gate at the two thresholds on every sample. It is an ordinary gate: open a sample and drag the crosshair, and the numbers below follow.",
            _ => "Puts three interval gates at the phase boundaries on every sample. They are ordinary gates: open a sample and drag their edges, and the numbers below follow. A gate counts the events between its edges; the fitted percentages above also split the overlap between phases, so the two can differ.",
        });
        let table = self.ana_gate_table(kind);
        let have = table.iter().any(|(_, r)| r.iter().any(|v| v.is_some()));
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new(if have { "Update the gates from this result" } else { "Make gates on all samples" }).strong()).fill(theme::ACCENT_DIM)).clicked() {
                self.status = match self.ana_make_gates(kind) {
                    Ok(m) => m,
                    Err(e) => format!("Could not make the gates: {e}"),
                };
            }
            ui.checkbox(&mut self.session.all_synced, "Moving a gate moves it in every sample").on_hover_text("The All Samples group's synchronisation. Off: a gate you move changes in that sample only and is marked edited.");
        });
        if !have {
            return;
        }
        let names = Self::ana_gate_names(kind);
        let mut open: Option<usize> = None;
        egui::Grid::new(("ana_gate_table", kind as u8)).striped(true).spacing([16.0, 3.0]).show(ui, |ui| {
            ui.label(RichText::new("Sample").size(11.0).color(theme::MUTED));
            for n in names {
                ui.label(RichText::new(format!("{n} %")).size(11.0).color(theme::MUTED));
            }
            ui.label("");
            ui.end_row();
            for (i, row) in &table {
                ui.label(self.session.samples()[*i].name.trim_end_matches(".fcs"));
                for v in row {
                    ui.label(RichText::new(v.map_or("—".to_string(), |v| format!("{v:.1}"))).monospace());
                }
                if ui.small_button("Open").on_hover_text("Show this sample's plot with the gates, in the Workstation").clicked() {
                    open = Some(*i);
                }
                ui.end_row();
            }
        });
        if ui.button("Copy table").clicked() {
            let mut t = format!("sample\t{}\n", names.join(" %\t") + " %");
            for (i, row) in &table {
                t += &format!("{}\t{}\n", self.session.samples()[*i].name, row.iter().map(|v| v.map_or(String::new(), |v| format!("{v:.2}"))).collect::<Vec<_>>().join("\t"));
            }
            ui.ctx().copy_text(t);
        }
        if let Some(i) = open {
            self.status = match self.ana_open_gates(kind, i) {
                Ok(()) => format!("{}: drag the gates on the plot; the analysis table follows.", self.session.samples()[i].name),
                Err(e) => e,
            };
        }
    }

    /// Show sample `i` in the Workstation on the axes the analysis' gates were made on.
    pub fn ana_open_gates(&mut self, kind: Kind, i: usize) -> Result<(), String> {
        let parent = self.ana_node(i).ok_or("this sample does not have that population")?;
        let first = Self::ana_gate_names(kind).first().copied().unwrap_or("");
        let gate = self.session.tree(i).children(parent).ok().and_then(|c| c.iter().copied().find(|&k| self.session.tree(i).name(k).ok() == Some(first))).and_then(|k| self.session.gate_of(i, k)).ok_or("this sample does not have the gates yet")?;
        let id = self.open_tab(i, parent)?;
        if let Some(w) = self.session.window_mut(id) {
            w.x_chan = gate.x_chan.clone();
            w.x_scale = gate.x_scale.clone();
            match &gate.y_chan {
                Some(y) => (w.y_chan, w.y_scale, w.kind) = (y.clone(), gate.y_scale.clone(), crate::density::PlotKind::Pseudocolor),
                None => w.kind = crate::density::PlotKind::Histogram,
            }
            w.tool = crate::drawing::Tool::Pointer;
        }
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.refit = true;
        }
        self.module = super::Module::Workstation;
        Ok(())
    }

    /// Left panel shared by the three screens: sample, population, channels, options, run button.
    fn ana_panel(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let n = self.session.samples().len();
        let (title, blurb) = match kind {
            Kind::Cycle => ("Cell cycle", "Fits the DNA-content histogram of a population with G0/G1 and G2/M peaks (G2/M at twice the G1 position) and an S-phase plateau between them."),
            Kind::Prolif => ("Proliferation", "Fits a dye-dilution histogram with one peak per generation, spaced one halving apart, and reports the division and proliferation indices."),
            Kind::Apoptosis => ("Apoptosis", "Splits Annexin V against a viability dye at the density valley of each axis into live, early apoptotic, late apoptotic and necrotic fractions."),
        };
        ui.label(RichText::new(title).strong().size(16.0));
        theme::caption(ui, blurb);
        theme::section(ui, "Data");
        let cur = self.session.samples()[self.ana.sample].name.clone();
        let mut chosen = self.ana.sample;
        egui::ComboBox::from_id_salt("ana_sample").width(260.0).selected_text(cur).show_ui(ui, |ui| {
            for i in 0..n {
                if ui.selectable_label(i == chosen, self.session.samples()[i].name.clone()).clicked() {
                    chosen = i;
                }
            }
        });
        if chosen != self.ana.sample {
            self.ana.sample = chosen;
            self.ana.restrict.clear();
        }
        let si = self.ana.sample;
        let paths: Vec<Vec<String>> = self.session.tree(si).self_and_descendants(GateTree::ROOT).unwrap_or_default().into_iter().skip(1).filter_map(|id| self.session.tree(si).path(id).ok()).collect();
        let cur = if self.ana.restrict.is_empty() { "All events".to_string() } else { self.ana.restrict.join(" ▸ ") };
        egui::ComboBox::from_id_salt("ana_pop").width(260.0).selected_text(format!("Population: {cur}")).show_ui(ui, |ui| {
            if ui.selectable_label(self.ana.restrict.is_empty(), "All events").clicked() {
                self.ana.restrict.clear();
            }
            for p in paths {
                if ui.selectable_label(self.ana.restrict == p, p.join(" ▸ ")).clicked() {
                    self.ana.restrict = p;
                }
            }
        });
        let chans = self.session.sample(si).channels.clone();
        let pick = |ui: &mut egui::Ui, id: &str, label: &str, cur: &mut String| {
            ui.horizontal(|ui| {
                ui.label(label);
                egui::ComboBox::from_id_salt(id.to_string()).width(170.0).selected_text(cur.clone()).show_ui(ui, |ui| {
                    for c in &chans {
                        if ui.selectable_label(c == cur, c.as_str()).clicked() {
                            *cur = c.clone();
                        }
                    }
                });
            });
        };
        match kind {
            Kind::Cycle => {
                pick(ui, "cc_chan", "DNA channel", &mut self.ana.cycle_channel);
                theme::section(ui, "Model");
                ui.radio_value(&mut self.ana.dj_fox, true, "Dean–Jett–Fox (G2/M width free)");
                ui.radio_value(&mut self.ana.dj_fox, false, "Watson pragmatic (G2/M width = 2 × G1)");
                ui.checkbox(&mut self.ana.use_hint, "Start from a G0/G1 peak position");
                if self.ana.use_hint {
                    ui.add(egui::DragValue::new(&mut self.ana.mu_hint).speed(100.0).range(0.0..=1e9));
                }
            }
            Kind::Prolif => {
                pick(ui, "pf_chan", "Dye channel", &mut self.ana.prolif_channel);
                ui.horizontal(|ui| {
                    ui.label("Most generations");
                    ui.add(egui::DragValue::new(&mut self.ana.generations).range(2..=12));
                });
            }
            Kind::Apoptosis => {
                pick(ui, "ap_ann", "Annexin V", &mut self.ana.annexin_channel);
                pick(ui, "ap_pi", "Viability dye", &mut self.ana.pi_channel);
                ui.horizontal(|ui| {
                    ui.label("arcsinh cofactor");
                    ui.add(egui::DragValue::new(&mut self.ana.cofactor).range(0.0..=100_000.0).speed(1.0));
                });
                theme::caption(ui, "Thresholds are found on the arcsinh scale and counted on the raw values. 0 = raw.");
            }
        }
        ui.add_space(10.0);
        if ui.add(egui::Button::new(RichText::new("Run").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(260.0, 32.0))).clicked() {
            match kind {
                Kind::Cycle => self.run_cell_cycle(),
                Kind::Prolif => self.run_proliferation(),
                Kind::Apoptosis => self.run_apoptosis(),
            }
        }
    }

    fn ana_frame(&mut self, ui: &mut egui::Ui, kind: Kind, body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load a sample first.").color(theme::MUTED)));
            return;
        }
        self.ana_sync();
        egui::SidePanel::left(format!("ana_cfg_{}", kind as u8)).resizable(false).exact_width(300.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.ana_panel(ui, kind));
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| body(self, ui));
        });
    }

    pub(super) fn cell_cycle_screen(&mut self, ui: &mut egui::Ui) {
        self.ana_frame(ui, Kind::Cycle, |app, ui| {
            let Some((label, r)) = app.ana.cycle.as_ref() else {
                ui.label(RichText::new("Pick the DNA channel and press Run.").color(theme::MUTED));
                return;
            };
            ui.label(RichText::new(format!("{label} · {} events fitted", r.n)).strong());
            let rect = plot_area(ui, 300.0);
            let (x, hist) = (&r.x, &r.hist);
            let ymax = hist.iter().cloned().fold(1.0, f64::max) * 1.08;
            let frame = Frame::new(rect, x[0], *x.last().unwrap(), 0.0, ymax);
            frame.draw_axes(ui);
            frame.bars(ui, x, hist, Color32::from_rgba_unmultiplied(0x60, 0x80, 0xA0, 150));
            frame.line(ui, x, &r.g1, G1, 1.6);
            frame.line(ui, x, &r.s, S_PHASE, 1.6);
            frame.line(ui, x, &r.g2, G2, 1.6);
            frame.line(ui, x, &r.total, FIT, 1.8);
            for (b, name) in [(r.boundaries[1], "G0/G1 | S"), (r.boundaries[2], "S | G2/M")] {
                frame.vline(ui, b, Color32::from_gray(150), name);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                phase_card(ui, "G0/G1", r.g1_pct, G1, Some(r.cv_g1));
                phase_card(ui, "S", r.s_pct, S_PHASE, None);
                phase_card(ui, "G2/M", r.g2_pct, G2, Some(r.cv_g2));
            });
            ui.add_space(6.0);
            ui.label(format!("G0/G1 peak {:.0} · G2/M peak {:.0} · R² {:.4} · RMSE {:.2}", r.mu_g1, r.mu_g2, r.r2, r.rmse));
            if r.r2 < 0.8 {
                ui.label(RichText::new(format!("Poor fit (R² {:.2}). This histogram does not look like a DNA-content distribution with a G0/G1 and a G2/M peak, so the phase percentages are not reliable. Check the channel, gate single cells first, or place the G0/G1 peak by hand.", r.r2)).color(theme::WARN));
            }
            if ui.button("Copy results").clicked() {
                let t = format!("sample\t{label}\nG0/G1 %\t{:.2}\nS %\t{:.2}\nG2/M %\t{:.2}\nCV G0/G1 %\t{:.2}\nCV G2/M %\t{:.2}\nR2\t{:.4}\nevents\t{}\n", r.g1_pct, r.s_pct, r.g2_pct, r.cv_g1, r.cv_g2, r.r2, r.n);
                ui.ctx().copy_text(t);
            }
            app.ana_gates_ui(ui, Kind::Cycle);
        });
    }

    pub(super) fn proliferation_screen(&mut self, ui: &mut egui::Ui) {
        self.ana_frame(ui, Kind::Prolif, |app, ui| {
            let Some((label, r)) = app.ana.prolif.as_ref() else {
                ui.label(RichText::new("Pick the dye channel and press Run.").color(theme::MUTED));
                return;
            };
            ui.label(RichText::new(format!("{label} · {} events fitted", r.n)).strong());
            let rect = plot_area(ui, 300.0);
            let ymax = r.hist.iter().cloned().fold(1.0, f64::max) * 1.08;
            let frame = Frame::new(rect, r.x[0], *r.x.last().unwrap(), 0.0, ymax);
            frame.draw_axes(ui);
            frame.bars(ui, &r.x, &r.hist, Color32::from_rgba_unmultiplied(0x60, 0x80, 0xA0, 150));
            frame.line(ui, &r.x, &r.total, FIT, 1.2);
            for (k, c) in r.curves.iter().enumerate() {
                frame.line(ui, &r.x, c, GEN[k % GEN.len()], 1.5);
            }
            ui.label(RichText::new("log10 of the dye signal; generation 0 (undivided) is the brightest peak on the right.").size(11.0).color(theme::MUTED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                big_number(ui, "Division index", format!("{:.2}", r.di));
                big_number(ui, "Proliferation index", format!("{:.2}", r.pi));
                big_number(ui, "% divided", format!("{:.1}", r.pct_divided));
            });
            ui.add_space(8.0);
            egui::Grid::new("gen_tbl").num_columns(3).spacing([24.0, 4.0]).show(ui, |ui| {
                for h in ["Generation", "Cells", "% of cells"] {
                    ui.label(RichText::new(h).color(theme::MUTED));
                }
                ui.end_row();
                for g in &r.generations {
                    ui.label(RichText::new(format!("● {}", g.gen)).color(GEN[g.gen % GEN.len()]));
                    ui.label(format!("{:.0}", g.count));
                    ui.label(format!("{:.2}", g.pct));
                    ui.end_row();
                }
            });
            theme::caption(ui, "Indices use the precursor cohort of each generation (cells ÷ 2^generation), so they are not biased by the doubling in cell number.");
            if ui.button("Copy results").clicked() {
                let mut t = format!("sample\t{label}\nDI\t{:.3}\nPI\t{:.3}\n% divided\t{:.2}\n", r.di, r.pi, r.pct_divided);
                for g in &r.generations {
                    t += &format!("generation {}\t{:.0}\t{:.2}\n", g.gen, g.count, g.pct);
                }
                ui.ctx().copy_text(t);
            }
        });
    }

    pub(super) fn apoptosis_screen(&mut self, ui: &mut egui::Ui) {
        self.ana_frame(ui, Kind::Apoptosis, |app, ui| {
            let cof = app.ana.cofactor;
            let (a_ch, p_ch) = (app.ana.annexin_channel.clone(), app.ana.pi_channel.clone());
            let Some((label, r, pts)) = app.ana.apop.as_ref() else {
                ui.label(RichText::new("Pick the two channels and press Run.").color(theme::MUTED));
                return;
            };
            ui.label(RichText::new(format!("{label} · {} events", r.n)).strong());
            let rect = plot_area(ui, 380.0);
            let tf = |v: f64| if cof > 0.0 { (v / cof).asinh() } else { v };
            let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for (x, y) in pts {
                let (a, b) = (tf(*x), tf(*y));
                x0 = x0.min(a);
                x1 = x1.max(a);
                y0 = y0.min(b);
                y1 = y1.max(b);
            }
            let frame = Frame::new(rect, x0, x1.max(x0 + 1e-9), y0, y1.max(y0 + 1e-9));
            frame.draw_axes(ui);
            let (tx, ty) = (tf(r.annexin_threshold), tf(r.pi_threshold));
            let painter = ui.painter_at(rect);
            for (x, y) in pts {
                let (a, b) = (*x >= r.annexin_threshold, *y >= r.pi_threshold);
                let c = match (a, b) {
                    (false, false) => Color32::from_rgba_unmultiplied(0x4C, 0xAF, 0x50, 110),
                    (true, false) => Color32::from_rgba_unmultiplied(0xF5, 0xA6, 0x23, 110),
                    (true, true) => Color32::from_rgba_unmultiplied(0xE0, 0x55, 0x55, 110),
                    (false, true) => Color32::from_rgba_unmultiplied(0x9B, 0x59, 0xB6, 110),
                };
                painter.circle_filled(frame.to_screen(tf(*x), tf(*y)), 1.3, c);
            }
            frame.vline(ui, tx, Color32::from_gray(200), "");
            frame.hline(ui, ty, Color32::from_gray(200));
            let q = r.quadrants;
            for (txt, fx, fy) in [("Necrotic", 0.12, 0.08), ("Late apoptotic", 0.88, 0.08), ("Live", 0.12, 0.88), ("Early apoptotic", 0.88, 0.88)] {
                let pct = match txt {
                    "Necrotic" => q.necrotic,
                    "Late apoptotic" => q.late,
                    "Live" => q.live,
                    _ => q.early,
                };
                painter.text(Pos2::new(rect.left() + fx * rect.width(), rect.top() + fy * rect.height()), Align2::CENTER_CENTER, format!("{txt}\n{pct:.1}%"), FontId::proportional(12.0), Color32::WHITE);
            }
            ui.label(RichText::new(format!("x: {a_ch}   y: {p_ch}   (arcsinh display, {} of {} events drawn)", pts.len(), r.n)).size(11.0).color(theme::MUTED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                phase_card(ui, "Live", q.live, G1, None);
                phase_card(ui, "Early apoptotic", q.early, S_PHASE, None);
                phase_card(ui, "Late apoptotic", q.late, G2, None);
                phase_card(ui, "Necrotic", q.necrotic, Color32::from_rgb(0x9B, 0x59, 0xB6), None);
            });
            ui.add_space(6.0);
            ui.label(format!("Thresholds (raw units): {a_ch} {:.1} · {p_ch} {:.1}", r.annexin_threshold, r.pi_threshold));
            if ui.button("Copy results").clicked() {
                let t = format!("sample\t{label}\nlive %\t{:.2}\nearly apoptotic %\t{:.2}\nlate apoptotic %\t{:.2}\nnecrotic %\t{:.2}\nannexin threshold\t{:.4}\nviability threshold\t{:.4}\nevents\t{}\n", q.live, q.early, q.late, q.necrotic, r.annexin_threshold, r.pi_threshold, r.n);
                ui.ctx().copy_text(t);
            }
            app.ana_gates_ui(ui, Kind::Apoptosis);
        });
    }
}

fn big_number(ui: &mut egui::Ui, label: &str, value: String) {
    egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).inner_margin(egui::Margin::symmetric(16.0, 10.0)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).size(11.0).color(theme::MUTED));
            ui.label(RichText::new(value).strong().size(22.0));
        });
    });
}

fn phase_card(ui: &mut egui::Ui, name: &str, pct: f64, colour: Color32, cv: Option<f64>) {
    egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).inner_margin(egui::Margin::symmetric(16.0, 10.0)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("● {name}")).size(12.0).color(colour));
            ui.label(RichText::new(format!("{pct:.1}%")).strong().size(22.0));
            if let Some(cv) = cv {
                ui.label(RichText::new(format!("CV {cv:.1}%")).size(11.0).color(theme::MUTED));
            }
        });
    });
}

pub(super) fn plot_area(ui: &mut egui::Ui, height: f32) -> Rect {
    let w = ui.available_width().min(900.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, height), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, theme::INPUT);
    rect
}

/// Data-to-screen mapping for one plot, with a margin for the axis labels.
pub(super) struct Frame {
    pub(super) rect: Rect,
    pub(super) x0: f64,
    pub(super) x1: f64,
    pub(super) y0: f64,
    pub(super) y1: f64,
    pub(super) x_ticks: bool,
}

impl Frame {
    pub(super) fn new(outer: Rect, x0: f64, x1: f64, y0: f64, y1: f64) -> Frame {
        Frame { rect: Rect::from_min_max(Pos2::new(outer.left() + 46.0, outer.top() + 10.0), Pos2::new(outer.right() - 12.0, outer.bottom() - 24.0)), x0, x1, y0, y1, x_ticks: true }
    }
    pub(super) fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        let fx = ((x - self.x0) / (self.x1 - self.x0)) as f32;
        let fy = ((y - self.y0) / (self.y1 - self.y0)) as f32;
        Pos2::new(self.rect.left() + fx * self.rect.width(), self.rect.bottom() - fy * self.rect.height())
    }
    pub(super) fn draw_axes(&self, ui: &egui::Ui) {
        let p = ui.painter();
        let grid = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20));
        for k in 0..=4 {
            let f = k as f32 / 4.0;
            let (y, x) = (self.rect.bottom() - f * self.rect.height(), self.rect.left() + f * self.rect.width());
            p.line_segment([Pos2::new(self.rect.left(), y), Pos2::new(self.rect.right(), y)], grid);
            p.text(Pos2::new(self.rect.left() - 4.0, y), Align2::RIGHT_CENTER, short(self.y0 + (self.y1 - self.y0) * f as f64), FontId::proportional(10.0), theme::MUTED);
            if self.x_ticks {
                p.text(Pos2::new(x, self.rect.bottom() + 3.0), Align2::CENTER_TOP, short(self.x0 + (self.x1 - self.x0) * f as f64), FontId::proportional(10.0), theme::MUTED);
            }
        }
        p.rect_stroke(self.rect, 0.0, Stroke::new(1.0_f32, Color32::from_gray(70)));
    }
    pub(super) fn bars(&self, ui: &egui::Ui, x: &[f64], y: &[f64], colour: Color32) {
        let p = ui.painter_at(self.rect);
        let w = ((self.rect.width() / x.len() as f32) - 0.5).max(1.0);
        for (xv, yv) in x.iter().zip(y) {
            let top = self.to_screen(*xv, *yv);
            let base = self.to_screen(*xv, self.y0);
            p.rect_filled(Rect::from_min_max(Pos2::new(top.x - w / 2.0, top.y), Pos2::new(top.x + w / 2.0, base.y)), 0.0, colour);
        }
    }
    pub(super) fn line(&self, ui: &egui::Ui, x: &[f64], y: &[f64], colour: Color32, width: f32) {
        let pts: Vec<Pos2> = x.iter().zip(y).map(|(a, b)| self.to_screen(*a, *b)).collect();
        ui.painter_at(self.rect).add(Shape::line(pts, Stroke::new(width, colour)));
    }
    pub(super) fn vline(&self, ui: &egui::Ui, x: f64, colour: Color32, label: &str) {
        if x < self.x0 || x > self.x1 {
            return;
        }
        let (a, b) = (self.to_screen(x, self.y0), self.to_screen(x, self.y1));
        let p = ui.painter_at(self.rect);
        p.line_segment([a, b], Stroke::new(1.0_f32, colour));
        if !label.is_empty() {
            p.text(Pos2::new(a.x + 3.0, self.rect.top() + 2.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), colour);
        }
    }
    pub(super) fn hline(&self, ui: &egui::Ui, y: f64, colour: Color32) {
        if y < self.y0 || y > self.y1 {
            return;
        }
        let (a, b) = (self.to_screen(self.x0, y), self.to_screen(self.x1, y));
        ui.painter_at(self.rect).line_segment([a, b], Stroke::new(1.0_f32, colour));
    }
}

fn short(v: f64) -> String {
    if v.abs() >= 10_000.0 {
        format!("{:.0}k", v / 1000.0)
    } else if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_guess_prefers_area_channels_and_known_dyes() {
        let c: Vec<String> = ["FSC-A", "PI-W", "PI-A", "FITC-A"].iter().map(|s| s.to_string()).collect();
        assert_eq!(guess_channel(&c, &["PI", "DAPI"]).as_deref(), Some("PI-A"));
        assert_eq!(guess_channel(&c, &["Hoechst", "FITC"]).as_deref(), Some("FITC-A"), "later hints are tried in order");
        assert_eq!(guess_channel(&c, &["CFSE"]), None);
    }
}
