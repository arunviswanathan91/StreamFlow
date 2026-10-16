//! Exporting a group: the events of an equivalent population in every member sample (FCS, or CSV with scale or channel
//! values, each sample separately or concatenated), the group's statistics table (CSV, Excel, PDF), and its layout
//! pages (PDF, PowerPoint, PNG; the buttons live on the layout screens, which this window opens for the group).

use super::{theme, App, Module};
use crate::office::{self, Cell};
use crate::session::ALL_SAMPLES;
use crate::stats_table::{compute, pop_label, to_csv, Metric, Table};
use eframe::egui::{self, RichText};
use sf_gating::{GateTree, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Fcs,
    /// Values as plotted (after each channel's default display scale).
    CsvScale,
    /// The channel numbers as recorded.
    CsvChannel,
}

impl DataFormat {
    pub const ALL: [DataFormat; 3] = [DataFormat::Fcs, DataFormat::CsvScale, DataFormat::CsvChannel];
    pub fn label(self) -> &'static str {
        match self {
            DataFormat::Fcs => "FCS file",
            DataFormat::CsvScale => "CSV, scale values",
            DataFormat::CsvChannel => "CSV, channel values",
        }
    }
    pub fn ext(self) -> &'static str {
        if self == DataFormat::Fcs {
            "fcs"
        } else {
            "csv"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFormat {
    Csv,
    Excel,
    Pdf,
}

pub struct State {
    pub open: bool,
    pub group: String,
    /// The population, as a path of names (empty = All Events).
    pub pop: Vec<String>,
    pub format: DataFormat,
    pub concat: bool,
}

impl Default for State {
    fn default() -> Self {
        State { open: false, group: ALL_SAMPLES.to_string(), pop: Vec::new(), format: DataFormat::Fcs, concat: false }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn file_stem(s: &str) -> String {
    s.trim_end_matches(".fcs").replace(['/', '\\', ':', ' ', '>', '*', '?', '"', '<', '|'], "_")
}

impl App {
    /// Population paths every member of the group has (All Events first), so an export names an equivalent
    /// population in each sample.
    pub fn common_populations(&self, group: &str) -> Vec<Vec<String>> {
        let members = self.session.member_indices(group);
        let Some(&first) = members.first() else { return Vec::new() };
        let t = self.session.tree(first);
        let mut out = vec![Vec::new()];
        for n in t.self_and_descendants(GateTree::ROOT).unwrap_or_default() {
            let path = t.path(n).unwrap_or_default();
            if path.is_empty() {
                continue;
            }
            let p: Vec<&str> = path.iter().map(String::as_str).collect();
            if members.iter().all(|&m| self.session.tree(m).find(&p).is_some()) {
                out.push(path);
            }
        }
        out
    }

    /// The files of a group export: `(file name, bytes)`; samples that lack the population are listed in the notes.
    pub fn group_data_files(&mut self, group: &str, pop: &[String], fmt: DataFormat, concat: bool) -> Result<(Vec<(String, Vec<u8>)>, Vec<String>), String> {
        let members = self.session.member_indices(group);
        if members.is_empty() {
            return Err(format!("The group '{group}' has no samples"));
        }
        let p: Vec<&str> = pop.iter().map(String::as_str).collect();
        let pop_name = if pop.is_empty() { "All Events".to_string() } else { pop.last().unwrap().clone() };
        let mut notes = Vec::new();
        // (sample index, node)
        let mut parts: Vec<(usize, NodeId)> = Vec::new();
        for &m in &members {
            let node = if p.is_empty() { Some(GateTree::ROOT) } else { self.session.tree(m).find(&p) };
            match node {
                Some(n) => parts.push((m, n)),
                None => notes.push(format!("{}: has no '{}', skipped", self.session.sample(m).name, pop_label(pop))),
            }
        }
        if parts.is_empty() {
            return Err(format!("No sample of '{group}' has '{}'", pop_label(pop)));
        }
        // channels must agree to concatenate; export the first sample's panel
        let channels = self.session.sample(parts[0].0).channels.clone();
        if concat {
            for &(m, _) in &parts[1..] {
                if self.session.sample(m).channels != channels {
                    return Err(format!("{} has different channels from {}; export the samples separately", self.session.sample(m).name, self.session.sample(parts[0].0).name));
                }
            }
        }
        let mut chunks: Vec<(usize, Vec<usize>)> = Vec::new();
        for &(m, n) in &parts {
            let rows = self.session.evaluation(m)?.masks.get(n).and_then(|x| x.as_ref()).map(|x| x.indices()).unwrap_or_default();
            chunks.push((m, rows));
        }
        let make = |this: &App, which: &[(usize, Vec<usize>)], sample_id: bool| -> Result<Vec<u8>, String> {
            let first = this.session.sample(which[0].0);
            let d = first.channels.len();
            let scales: Vec<_> = (0..d).map(|i| crate::session::default_scale(&first.channels[i], first.ranges[i]).build().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
            match fmt {
                DataFormat::Fcs => {
                    let mut params: Vec<sf_fcs::Param> = (0..d).map(|i| sf_fcs::Param { name: first.channels[i].clone(), stain: first.labels.get(i).cloned().unwrap_or_default(), range: first.ranges[i], bits: 32, exponent: "0,0".into() }).collect();
                    if sample_id {
                        params.push(sf_fcs::Param { name: "SampleID".into(), stain: String::new(), range: (which.len() + 1) as f64, bits: 32, exponent: "0,0".into() });
                    }
                    let mut ev: Vec<f32> = Vec::new();
                    for (k, (m, rows)) in which.iter().enumerate() {
                        let s = this.session.sample(*m);
                        for r in rows {
                            ev.extend_from_slice(&s.events[r * d..(r + 1) * d]);
                            if sample_id {
                                ev.push((k + 1) as f32);
                            }
                        }
                    }
                    let mut extra: Vec<(String, String)> = first.keywords.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    extra.sort();
                    sf_fcs::write_fcs(&params, &ev, &extra)
                }
                DataFormat::CsvScale | DataFormat::CsvChannel => {
                    let mut out = String::new();
                    let mut head: Vec<String> = first.channels.iter().map(|c| csv_field(c)).collect();
                    if sample_id {
                        head.push("SampleID".into());
                    }
                    out.push_str(&head.join(","));
                    out.push('\n');
                    for (k, (m, rows)) in which.iter().enumerate() {
                        let s = this.session.sample(*m);
                        for r in rows {
                            let mut line: Vec<String> = (0..d)
                                .map(|c| {
                                    let v = s.events[r * d + c] as f64;
                                    if fmt == DataFormat::CsvScale {
                                        format!("{}", scales[c].forward(v))
                                    } else {
                                        format!("{v}")
                                    }
                                })
                                .collect();
                            if sample_id {
                                line.push((k + 1).to_string());
                            }
                            out.push_str(&line.join(","));
                            out.push('\n');
                        }
                    }
                    Ok(out.into_bytes())
                }
            }
        };
        let mut files = Vec::new();
        if concat {
            let name = format!("{}_{}_concatenated.{}", file_stem(group), file_stem(&pop_name), fmt.ext());
            files.push((name, make(self, &chunks, true)?));
            let map: Vec<String> = parts.iter().enumerate().map(|(k, (m, _))| format!("{} = {}", k + 1, self.session.sample(*m).name)).collect();
            notes.push(format!("SampleID: {}", map.join("; ")));
        } else {
            for c in &chunks {
                let name = format!("{}_{}.{}", file_stem(&self.session.sample(c.0).name), file_stem(&pop_name), fmt.ext());
                files.push((name, make(self, std::slice::from_ref(c), false)?));
            }
        }
        Ok((files, notes))
    }

    /// The group's statistics: every population of every member, with the metrics chosen on the Statistics screen.
    pub fn group_table(&mut self, group: &str) -> Result<Table, String> {
        let members = self.session.member_indices(group);
        if members.is_empty() {
            return Err(format!("The group '{group}' has no samples"));
        }
        let metrics: Vec<Metric> = self.stats.metrics.iter().filter(|(_, on)| *on).map(|(m, _)| *m).collect();
        if metrics.is_empty() {
            return Err("Choose at least one statistic on the Statistics screen first".into());
        }
        let ch = if self.stats.channel.is_empty() { self.session.sample(members[0]).channels.first().cloned().unwrap_or_default() } else { self.stats.channel.clone() };
        Ok(compute(&mut self.session, &members, &metrics, Some(&ch)))
    }

    pub fn group_table_bytes(&mut self, group: &str, fmt: TableFormat) -> Result<Vec<u8>, String> {
        let t = self.group_table(group)?;
        Ok(match fmt {
            TableFormat::Csv => to_csv(&t).into_bytes(),
            TableFormat::Excel => office::xlsx(&format!("{group} statistics"), &crate::table_export::table_rows(&t)),
            TableFormat::Pdf => crate::figure::to_pdf(&crate::table_export::table_pages(&t, &format!("{group}: statistics")), group),
        })
    }

    /// Built layout pages as a PowerPoint deck: one picture per slide at the chosen resolution.
    pub fn pages_pptx(&self, pages: &[crate::figure::Figure], title: &str) -> Result<Vec<u8>, String> {
        if pages.is_empty() {
            return Err("Build the layout first".into());
        }
        let dpi = self.pexp.set.dpi.clamp(72.0, 300.0);
        let mut slides = Vec::new();
        for (i, f) in pages.iter().enumerate() {
            let r = crate::raster::rasterize(f, dpi, false)?;
            slides.push(office::Slide { png: crate::raster::to_png(&r, dpi)?, title: format!("{title} — page {} of {}", i + 1, pages.len()), width_pt: f.width, height_pt: f.height });
        }
        Ok(office::pptx(&slides))
    }

    pub(crate) fn group_export_window(&mut self, ctx: &egui::Context) {
        if !self.gx.open {
            return;
        }
        let mut open = true;
        let groups: Vec<String> = std::iter::once(ALL_SAMPLES.to_string()).chain(self.session.groups.iter().map(|g| g.name.clone())).collect();
        if !groups.contains(&self.gx.group) {
            self.gx.group = ALL_SAMPLES.to_string();
        }
        let pops = self.common_populations(&self.gx.group);
        if !pops.contains(&self.gx.pop) {
            self.gx.pop = Vec::new();
        }
        let n_members = self.session.member_indices(&self.gx.group).len();
        let mut act: Option<&'static str> = None;
        let mut tfmt = TableFormat::Csv;
        egui::Window::new("Export group").open(&mut open).collapsible(false).default_width(470.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Group");
                egui::ComboBox::from_id_salt("gx_group").width(200.0).selected_text(format!("{} ({n_members})", self.gx.group)).show_ui(ui, |ui| {
                    for g in &groups {
                        ui.selectable_value(&mut self.gx.group, g.clone(), g);
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Events").strong().color(theme::MUTED).size(12.0));
            theme::caption(ui, "The same population in every sample, saved as new files.");
            ui.horizontal(|ui| {
                ui.label("Population");
                egui::ComboBox::from_id_salt("gx_pop").width(260.0).selected_text(pop_label(&self.gx.pop)).show_ui(ui, |ui| {
                    for p in &pops {
                        ui.selectable_value(&mut self.gx.pop, p.clone(), pop_label(p));
                    }
                });
            });
            ui.horizontal(|ui| {
                for f in DataFormat::ALL {
                    ui.selectable_value(&mut self.gx.format, f, f.label());
                }
            });
            ui.checkbox(&mut self.gx.concat, "Concatenate into one file (adds a SampleID column)");
            if ui.button(RichText::new(if self.gx.concat { "Export concatenated file…" } else { "Export one file per sample…" }).strong()).clicked() {
                act = Some("data");
            }
            ui.separator();
            ui.label(RichText::new("Statistics table").strong().color(theme::MUTED).size(12.0));
            theme::caption(ui, "Every population of every sample in the group, with the statistics ticked on the Statistics screen.");
            ui.horizontal(|ui| {
                for (label, f) in [("CSV…", TableFormat::Csv), ("Excel…", TableFormat::Excel), ("PDF…", TableFormat::Pdf)] {
                    if ui.button(label).clicked() {
                        tfmt = f;
                        act = Some("table");
                    }
                }
                if ui.small_button("Choose statistics").clicked() {
                    act = Some("stats");
                }
            });
            ui.separator();
            ui.label(RichText::new("Layout").strong().color(theme::MUTED).size(12.0));
            theme::caption(ui, "A page of plots for the group's samples, saved as PDF, PowerPoint or PNG from the Layout screen.");
            if ui.button("Open Layout with this group…").clicked() {
                act = Some("layout");
            }
        });
        let (group, pop, fmt, concat) = (self.gx.group.clone(), self.gx.pop.clone(), self.gx.format, self.gx.concat);
        match act {
            Some("data") => self.save_group_data(&group, &pop, fmt, concat),
            Some("table") => self.save_group_table(&group, tfmt),
            Some("stats") => {
                self.module = Module::Statistics;
            }
            Some("layout") => {
                self.use_group_in_layout(&group);
                self.module = Module::Layout;
            }
            _ => {}
        }
        self.gx.open = open;
    }

    /// Open the group export window on a group (command line, screenshots).
    pub fn open_group_export(&mut self, group: &str) {
        self.gx.group = group.to_string();
        self.gx.open = true;
    }

    /// Select exactly this group's samples for the grid layout.
    pub fn use_group_in_layout(&mut self, group: &str) {
        let members = self.session.member_indices(group);
        let n = self.session.samples().len();
        self.lay.sel = (0..n).map(|i| members.contains(&i)).collect();
        self.lay.tab = super::layout_view::LayTab::Grid;
        self.status = format!("Layout: the {} sample(s) of '{group}' are selected.", members.len());
    }

    fn save_group_data(&mut self, group: &str, pop: &[String], fmt: DataFormat, concat: bool) {
        let (files, notes) = match self.group_data_files(group, pop, fmt, concat) {
            Ok(x) => x,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let Some(dir) = rfd::FileDialog::new().set_title("Choose a folder for the exported files").pick_folder() else { return };
        let clash = files.iter().filter(|(n, _)| dir.join(n).exists()).count();
        if clash > 0 {
            self.status = format!("{clash} file(s) with these names already exist in {}; choose another folder (nothing was written).", dir.display());
            return;
        }
        let mut done = 0;
        for (name, bytes) in &files {
            if let Err(e) = std::fs::write(dir.join(name), bytes) {
                self.status = format!("Could not write {name}: {e} ({done} file(s) written)");
                return;
            }
            done += 1;
        }
        self.status = format!("Exported {done} file(s) to {}.{}", dir.display(), if notes.is_empty() { String::new() } else { format!(" {}", notes.join(" ")) });
    }

    fn save_group_table(&mut self, group: &str, fmt: TableFormat) {
        let (ext, label) = match fmt {
            TableFormat::Csv => ("csv", "CSV"),
            TableFormat::Excel => ("xlsx", "Excel"),
            TableFormat::Pdf => ("pdf", "PDF"),
        };
        let bytes = match self.group_table_bytes(group, fmt) {
            Ok(b) => b,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let Some(p) = rfd::FileDialog::new().add_filter(label, &[ext]).set_file_name(format!("{}_statistics.{ext}", file_stem(group))).save_file() else { return };
        self.status = match std::fs::write(&p, bytes) {
            Ok(()) => format!("Saved {}.", p.display()),
            Err(e) => format!("Could not save: {e}"),
        };
    }
}

#[allow(dead_code)]
fn _cell_is_used(_: Cell) {}
