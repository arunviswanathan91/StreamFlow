//! Dim. Reduction and Clustering screens. They share one state: the pooled events, the embedding, the
//! clusters and the marker rules. Clusters are PROPOSALS until approved; approving turns them into
//! populations of the gating tree (undoable, whole-file for FlowSOM).

use super::graph::Mapper;
use super::{theme, App};
use crate::density::{bin, render, Range};
use crate::drawing::{to_gate, Drawing, Shape, Tool};
use crate::embed::{self, ClusterMethod, ClusterRequest, ClusterResult, EmbedInput, EmbedRequest, Embedding, FeatureTransform, Method, NameMethod, Scope};
use crate::jobs::{self, Job};
use crate::sample::{COMP_PREFIX, UNMIX_PREFIX};
use crate::session::short_name;
use crate::spectral_app::spectral_detectors;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions};
use sf_core::cluster::Scale;
use sf_core::discovery::Rule;
use sf_core::stats;
use std::collections::BTreeMap;

const PALETTE: [Color32; 12] = [
    Color32::from_rgb(0x1F, 0x77, 0xB4),
    Color32::from_rgb(0xFF, 0x7F, 0x0E),
    Color32::from_rgb(0x2C, 0xA0, 0x2C),
    Color32::from_rgb(0xD6, 0x27, 0x28),
    Color32::from_rgb(0x94, 0x67, 0xBD),
    Color32::from_rgb(0x8C, 0x56, 0x4B),
    Color32::from_rgb(0xE3, 0x77, 0xC2),
    Color32::from_rgb(0x7F, 0x7F, 0x7F),
    Color32::from_rgb(0xBC, 0xBD, 0x22),
    Color32::from_rgb(0x17, 0xBE, 0xCF),
    Color32::from_rgb(0xAE, 0xC7, 0xE8),
    Color32::from_rgb(0xFF, 0xBB, 0x78),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBy {
    Density,
    Marker(usize),
    Cluster,
    Sample,
    Rule(usize),
}

pub struct RuleUi {
    pub name: String,
    pub clauses: Vec<(String, bool)>,
}

pub struct State {
    pub sel: Vec<bool>,
    pub per_sample: usize,
    pub features: Vec<(String, bool)>,
    features_for: Option<usize>,
    pub cofactor: f64,
    pub umap: bool,
    pub n_neighbors: usize,
    pub min_dist: f64,
    pub perplexity: f64,
    pub seed: u64,
    pub restrict: Vec<String>,
    pub cluster_method: ClusterMethod,
    pub k: usize,
    pub name_method: NameMethod,
    pub thresholds: BTreeMap<String, f64>,
    pub rules: Vec<RuleUi>,
    pub input: Option<EmbedInput>,
    pub emb: Option<Embedding>,
    pub clus: Option<ClusterResult>,
    pub names: Vec<String>,
    pub picked: Vec<bool>,
    pub color: ColorBy,
    embed_job: Option<Job<Embedding>>,
    cluster_job: Option<Job<ClusterResult>>,
    range: Option<Range>,
    tex: Option<TextureHandle>,
    tex_key: u64,
    pub plot_rect: Option<Rect>,
    pub tool: Tool,
    pub drawing: Drawing,
    pub region_count: usize,
    new_rule: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            sel: vec![],
            per_sample: 3000,
            features: vec![],
            features_for: None,
            cofactor: 150.0,
            umap: true,
            n_neighbors: 15,
            min_dist: 0.1,
            perplexity: 30.0,
            seed: 42,
            restrict: vec![],
            cluster_method: ClusterMethod::FlowSom,
            k: 8,
            name_method: NameMethod::ZScore,
            thresholds: BTreeMap::new(),
            rules: vec![],
            input: None,
            emb: None,
            clus: None,
            names: vec![],
            picked: vec![],
            color: ColorBy::Density,
            embed_job: None,
            cluster_job: None,
            range: None,
            tex: None,
            tex_key: 0,
            plot_rect: None,
            tool: Tool::Pointer,
            drawing: Drawing::default(),
            region_count: 0,
            new_rule: String::new(),
        }
    }
}

impl State {
    pub fn is_busy(&self) -> bool {
        self.embed_job.is_some() || self.cluster_job.is_some()
    }
    pub fn job_label(&self) -> Option<String> {
        self.embed_job.as_ref().map(|j| format!("{} {}s", j.label, j.elapsed().as_secs())).or_else(|| self.cluster_job.as_ref().map(|j| format!("{} {}s", j.label, j.elapsed().as_secs())))
    }
}

/// Default markers: unmixed channels if the sample has them, else compensated, else raw fluorescence.
pub fn default_features(s: &crate::sample::LoadedSample) -> Vec<String> {
    let unmixed: Vec<String> = s.channels.iter().filter(|c| c.starts_with(UNMIX_PREFIX) && !c.ends_with("Residual") && !c.ends_with("-AF")).cloned().collect();
    if !unmixed.is_empty() {
        return unmixed;
    }
    let comp: Vec<String> = s.channels.iter().filter(|c| c.starts_with(COMP_PREFIX)).cloned().collect();
    if !comp.is_empty() {
        return comp;
    }
    spectral_detectors(s)
}

impl App {
    /// Collect finished background jobs (called every frame by the Dim. Reduction and Clustering screens).
    pub fn poll_jobs(&mut self) {
        self.disc_sync();
    }

    fn disc_sync(&mut self) {
        let n = self.session.samples().len();
        self.disc.sel.resize(n, true);
        let first = (0..n).find(|&i| self.disc.sel[i]);
        if first != self.disc.features_for {
            self.disc.features_for = first;
            if let Some(i) = first {
                let marks = default_features(self.session.sample(i));
                let all: Vec<String> = self.session.sample(i).channels.clone();
                let mut f: Vec<(String, bool)> = all.iter().filter(|c| !c.to_ascii_uppercase().contains("FSC") && !c.to_ascii_uppercase().contains("SSC") && !c.to_ascii_uppercase().contains("TIME") && !c.ends_with("Residual")).map(|c| (c.clone(), marks.contains(c))).collect();
                f.retain(|(c, _)| c.ends_with("-A") || c.starts_with(COMP_PREFIX) || c.starts_with(UNMIX_PREFIX));
                self.disc.features = f;
            }
        }
        // collect finished jobs
        if let Some(job) = self.disc.embed_job.as_mut() {
            if let Some(r) = job.poll() {
                self.disc.embed_job = None;
                match r {
                    Ok(e) => {
                        self.status = format!("Embedded {} events with {}.", e.input.n(), e.input.request.method.label());
                        self.disc.thresholds = embed::default_thresholds(&e.input);
                        self.disc.input = Some(e.input.clone());
                        self.disc.range = None;
                        self.disc.tex = None;
                        self.disc.clus = None;
                        self.disc.emb = Some(e);
                    }
                    Err(e) => self.status = format!("Embedding failed: {e}"),
                }
            }
        }
        if let Some(job) = self.disc.cluster_job.as_mut() {
            if let Some(r) = job.poll() {
                self.disc.cluster_job = None;
                match r {
                    Ok(c) => {
                        self.status = format!("{} found {} clusters.", c.actual.label(), c.n_clusters);
                        self.disc.picked = vec![false; c.n_clusters];
                        self.disc.clus = Some(c);
                        self.disc_rename();
                        self.disc.tex = None;
                    }
                    Err(e) => self.status = format!("Clustering failed: {e}"),
                }
            }
        }
    }

    fn tag_of(&self) -> impl Fn(&str) -> Option<String> {
        |c: &str| Some(short_name(c))
    }

    pub(crate) fn disc_request(&self) -> Result<EmbedRequest, String> {
        let samples: Vec<usize> = (0..self.session.samples().len()).filter(|&i| self.disc.sel.get(i).copied().unwrap_or(false)).collect();
        let features: Vec<String> = self.disc.features.iter().filter(|(_, on)| *on).map(|(c, _)| c.clone()).collect();
        Ok(EmbedRequest {
            samples,
            restrict: self.disc.restrict.clone(),
            per_sample: self.disc.per_sample,
            features,
            transform: if self.disc.cofactor > 0.0 { FeatureTransform::Arcsinh(self.disc.cofactor) } else { FeatureTransform::None },
            method: if self.disc.umap { Method::Umap { n_neighbors: self.disc.n_neighbors, min_dist: self.disc.min_dist } } else { Method::Tsne { perplexity: self.disc.perplexity } },
            seed: self.disc.seed,
        })
    }

    /// Pool the events and start the embedding on a worker thread.
    pub fn disc_run_embedding(&mut self) {
        if self.disc.is_busy() {
            return;
        }
        let req = match self.disc_request() {
            Ok(r) => r,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        match embed::build_input(&mut self.session, &req) {
            Ok(input) => {
                for n in &input.notes {
                    self.session.log.push(n.clone());
                }
                let label = input.request.method.label().to_string();
                self.disc.embed_job = Some(jobs::spawn(label, move || embed::run_embedding(&input).map(|xy| Embedding { input, xy })));
                self.status = "Embedding… (the interface stays usable)".into();
            }
            Err(e) => self.status = format!("Cannot embed: {e}"),
        }
    }

    /// Pool the events (no embedding) so clusters can be computed on their own.
    pub fn disc_pool_only(&mut self) -> bool {
        let req = match self.disc_request() {
            Ok(r) => r,
            Err(e) => {
                self.status = e;
                return false;
            }
        };
        match embed::build_input(&mut self.session, &req) {
            Ok(input) => {
                for n in &input.notes {
                    self.session.log.push(n.clone());
                }
                self.disc.thresholds = embed::default_thresholds(&input);
                self.disc.input = Some(input);
                self.disc.clus = None;
                true
            }
            Err(e) => {
                self.status = format!("Cannot pool events: {e}");
                false
            }
        }
    }

    pub fn disc_run_clustering(&mut self) {
        if self.disc.is_busy() {
            return;
        }
        if self.disc.input.is_none() && !self.disc_pool_only() {
            return;
        }
        let input = self.disc.input.clone().unwrap();
        let req = ClusterRequest { method: self.disc.cluster_method, k: self.disc.k, scale: Scale::ZScore, seed: self.disc.seed };
        let label = req.method.label().to_string();
        self.disc.cluster_job = Some(jobs::spawn(label, move || embed::cluster(&input, &req)));
        self.status = "Clustering…".into();
    }

    pub(crate) fn disc_rename(&mut self) {
        let (Some(input), Some(res)) = (&self.disc.input, &self.disc.clus) else { return };
        let rules: Vec<Rule> = self.disc.rules.iter().filter(|r| !r.clauses.is_empty()).map(|r| Rule { name: r.name.clone(), clauses: r.clauses.iter().map(|(c, p)| sf_core::discovery::Clause { channel: c.clone(), positive: *p }).collect() }).collect();
        let tag = self.tag_of();
        self.disc.names = embed::name_clusters(input, res, self.disc.name_method, &self.disc.thresholds, &rules, &tag);
    }

    fn rules_core(&self) -> Vec<Rule> {
        self.disc.rules.iter().filter(|r| !r.clauses.is_empty()).map(|r| Rule { name: r.name.clone(), clauses: r.clauses.iter().map(|(c, p)| sf_core::discovery::Clause { channel: c.clone(), positive: *p }).collect() }).collect()
    }

    // ---- shared configuration panel ---------------------------------------------------------------

    fn disc_config(&mut self, ui: &mut egui::Ui, with_embedding: bool) {
        ui.label(RichText::new(if with_embedding { "Dimensionality reduction" } else { "Clustering" }).strong().size(16.0));
        theme::caption(ui, if with_embedding { "Pool equal numbers of events from each sample, embed them, then discover populations." } else { "Cluster the pooled events on their marker values; approve clusters as populations." });
        theme::section(ui, "Samples");
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                self.disc.sel.iter_mut().for_each(|s| *s = true);
            }
            if ui.small_button("None").clicked() {
                self.disc.sel.iter_mut().for_each(|s| *s = false);
            }
        });
        egui::ScrollArea::vertical().id_salt("disc_samples").max_height(130.0).show(ui, |ui| {
            for i in 0..self.session.samples().len() {
                let name = self.session.samples()[i].name.clone();
                ui.checkbox(&mut self.disc.sel[i], name);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Events per sample");
            ui.add(egui::DragValue::new(&mut self.disc.per_sample).range(100..=200_000).speed(100.0));
        });
        // restrict to a population (by path) present in the first selected sample
        let first = self.disc.features_for;
        if let Some(si) = first {
            let paths: Vec<Vec<String>> = self.session.tree(si).self_and_descendants(sf_gating::GateTree::ROOT).unwrap_or_default().into_iter().skip(1).filter_map(|n| self.session.tree(si).path(n).ok()).collect();
            let cur = if self.disc.restrict.is_empty() { "All events".to_string() } else { self.disc.restrict.join(" ▸ ") };
            egui::ComboBox::from_id_salt("disc_restrict").width(240.0).selected_text(format!("Use: {cur}")).show_ui(ui, |ui| {
                if ui.selectable_label(self.disc.restrict.is_empty(), "All events").clicked() {
                    self.disc.restrict.clear();
                }
                for p in paths {
                    if ui.selectable_label(self.disc.restrict == p, p.join(" ▸ ")).clicked() {
                        self.disc.restrict = p;
                    }
                }
            });
        }
        theme::section(ui, "Markers");
        egui::ScrollArea::vertical().id_salt("disc_feats").max_height(150.0).show(ui, |ui| {
            for (c, on) in self.disc.features.iter_mut() {
                ui.checkbox(on, c.as_str());
            }
        });
        ui.horizontal(|ui| {
            ui.label("arcsinh cofactor");
            ui.add(egui::DragValue::new(&mut self.disc.cofactor).range(0.0..=100_000.0).speed(1.0));
        });
        theme::caption(ui, "0 = no transform");
        if with_embedding {
            theme::section(ui, "Method");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.disc.umap, true, "UMAP");
                ui.radio_value(&mut self.disc.umap, false, "t-SNE");
            });
            if self.disc.umap {
                ui.horizontal(|ui| {
                    ui.label("neighbours");
                    ui.add(egui::DragValue::new(&mut self.disc.n_neighbors).range(3..=200));
                    ui.label("min dist");
                    ui.add(egui::DragValue::new(&mut self.disc.min_dist).range(0.0..=1.0).speed(0.01));
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label("perplexity");
                    ui.add(egui::DragValue::new(&mut self.disc.perplexity).range(2.0..=200.0).speed(0.5));
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label("seed");
            ui.add(egui::DragValue::new(&mut self.disc.seed));
        });
        ui.add_space(6.0);
        if with_embedding {
            let busy = self.disc.embed_job.is_some();
            let label = if busy { format!("Running… {}", self.disc.job_label().unwrap_or_default()) } else { "Run embedding".to_string() };
            if ui.add_enabled(!self.disc.is_busy(), egui::Button::new(RichText::new(label).strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(250.0, 32.0))).clicked() {
                self.disc_run_embedding();
            }
        } else if ui.add_enabled(!self.disc.is_busy(), egui::Button::new(RichText::new("Pool events").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(250.0, 30.0))).clicked() {
            self.disc_pool_only();
            if self.disc.input.is_some() {
                self.status = format!("Pooled {} events.", self.disc.input.as_ref().unwrap().n());
            }
        }
        if let Some(i) = &self.disc.input {
            theme::caption(ui, format!("{} events pooled from {} sample(s)", i.n(), i.request.samples.len()));
        }
    }

    // ---- clustering + naming + rules panel --------------------------------------------------------

    fn disc_discovery(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Clusters");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("clus_method").width(120.0).selected_text(self.disc.cluster_method.label()).show_ui(ui, |ui| {
                for m in ClusterMethod::ALL {
                    ui.selectable_value(&mut self.disc.cluster_method, m, m.label());
                }
            });
            if self.disc.cluster_method.uses_k() {
                ui.label("k");
                ui.add(egui::DragValue::new(&mut self.disc.k).range(2..=100));
            }
        });
        let has_input = self.disc.input.is_some();
        if ui.add_enabled(!self.disc.is_busy(), egui::Button::new(if has_input { "Cluster the pooled events" } else { "Pool + cluster" }).min_size(egui::vec2(240.0, 28.0))).clicked() {
            self.disc_run_clustering();
        }
        let mut renamed = false;
        egui::ComboBox::from_id_salt("name_method").width(240.0).selected_text(self.disc.name_method.label()).show_ui(ui, |ui| {
            for m in NameMethod::ALL {
                renamed |= ui.selectable_value(&mut self.disc.name_method, m, m.label()).changed();
            }
        });
        if renamed {
            self.disc_rename();
        }
        let n_clusters = self.disc.clus.as_ref().map_or(0, |c| c.n_clusters);
        if n_clusters > 0 {
            let counts = self.disc.clus.as_ref().unwrap().counts.clone();
            let total: usize = counts.iter().sum::<usize>().max(1);
            let actual = self.disc.clus.as_ref().unwrap().actual;
            theme::caption(ui, format!("{} · named by marker medians · unclustered events are not listed", actual.label()));
            egui::ScrollArea::vertical().id_salt("clus_list").max_height(230.0).show(ui, |ui| {
                for c in 0..n_clusters {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, PALETTE[c % PALETTE.len()]);
                        if let Some(p) = self.disc.picked.get_mut(c) {
                            ui.checkbox(p, "");
                        }
                        if let Some(nm) = self.disc.names.get_mut(c) {
                            ui.add(egui::TextEdit::singleline(nm).desired_width(120.0));
                        }
                        ui.label(RichText::new(format!("{}  {:.1}%", counts[c], 100.0 * counts[c] as f64 / total as f64)).monospace().size(11.0).color(theme::MUTED));
                    });
                }
            });
            ui.horizontal(|ui| {
                if ui.small_button("select all").clicked() {
                    self.disc.picked.iter_mut().for_each(|p| *p = true);
                }
                if ui.small_button("none").clicked() {
                    self.disc.picked.iter_mut().for_each(|p| *p = false);
                }
                if ui.small_button("colour by cluster").clicked() {
                    self.disc.color = ColorBy::Cluster;
                    self.disc.tex = None;
                }
            });
            let can_full = self.disc.clus.as_ref().unwrap().model.is_some();
            if ui.add(egui::Button::new("Approve selected → populations (pooled events)").min_size(egui::vec2(260.0, 26.0))).clicked() {
                self.disc_approve_clusters(Scope::Embedded);
            }
            if ui.add_enabled(can_full, egui::Button::new("Approve selected → populations (whole files)").min_size(egui::vec2(260.0, 26.0))).on_hover_text(if can_full { "Every event of each sample is assigned by the trained FlowSOM map" } else { "Only FlowSOM can place events that were not clustered" }).clicked() {
                self.disc_approve_clusters(Scope::FullFile);
            }
        }
        // ---- rules (label-based discovery)
        theme::section(ui, "Marker rules");
        theme::caption(ui, "A rule is an AND of marker +/- calls against the thresholds below. It is evaluated on events, independent of clustering.");
        if let Some(input) = self.disc.input.clone() {
            egui::CollapsingHeader::new("Thresholds (transformed units)").default_open(false).show(ui, |ui| {
                for (j, f) in input.request.features.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(short_name(f)).size(12.0));
                        let t = self.disc.thresholds.entry(f.clone()).or_insert_with(|| stats::otsu(&input.column(j)));
                        let changed = ui.add(egui::DragValue::new(t).speed(0.02)).changed();
                        if ui.small_button("auto").on_hover_text("Otsu split of this marker").clicked() {
                            *t = stats::otsu(&input.column(j));
                            renamed = true;
                        }
                        renamed |= changed;
                    });
                }
            });
            if renamed {
                self.disc_rename();
            }
            let feats = input.request.features.clone();
            let mut remove = None;
            for (ri, r) in self.disc.rules.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(90.0));
                    ui.label("=");
                    let mut drop_clause = None;
                    for (ci, (c, pos)) in r.clauses.iter_mut().enumerate() {
                        if theme::toggle(ui, *pos, format!("{}{}", short_name(c), if *pos { "+" } else { "−" })).clicked() {
                            *pos = !*pos;
                        }
                        if ui.add(egui::Button::new(RichText::new("×").color(theme::MUTED)).frame(false)).clicked() {
                            drop_clause = Some(ci);
                        }
                    }
                    if let Some(ci) = drop_clause {
                        r.clauses.remove(ci);
                    }
                    ui.menu_button("＋ marker", |ui| {
                        for f in &feats {
                            if ui.button(short_name(f)).clicked() {
                                r.clauses.push((f.clone(), true));
                                ui.close_menu();
                            }
                        }
                    });
                    if ui.small_button("delete rule").clicked() {
                        remove = Some(ri);
                    }
                });
            }
            if let Some(ri) = remove {
                self.disc.rules.remove(ri);
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.disc.new_rule).hint_text("new rule name").desired_width(120.0));
                if ui.button("Add rule").clicked() && !self.disc.new_rule.trim().is_empty() {
                    self.disc.rules.push(RuleUi { name: self.disc.new_rule.trim().to_string(), clauses: vec![] });
                    self.disc.new_rule.clear();
                }
            });
            let rules = self.rules_core();
            if !rules.is_empty() {
                let counts = sf_core::discovery::counts(&input.feats, input.n(), &input.request.features, &rules, &self.disc.thresholds);
                for (i, (name, c)) in counts.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{name}: {c} events ({:.1}%)", 100.0 * *c as f64 / input.n().max(1) as f64)).size(12.0));
                        if ui.small_button("colour").clicked() {
                            self.disc.color = ColorBy::Rule(i);
                            self.disc.tex = None;
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Approve all rules (pooled events)").clicked() {
                        self.disc_approve_rules(Scope::Embedded);
                    }
                    if ui.button("Approve all rules (whole files)").clicked() {
                        self.disc_approve_rules(Scope::FullFile);
                    }
                });
            }
        } else {
            theme::caption(ui, "Pool events first.");
        }
    }

    pub fn disc_approve_clusters(&mut self, scope: Scope) {
        let (Some(input), Some(res)) = (self.disc.input.clone(), self.disc.clus.clone()) else { return };
        let which = self.disc.picked.clone();
        if !which.iter().any(|&p| p) {
            self.status = "Tick the clusters to approve first.".into();
            return;
        }
        self.status = match embed::approve_clusters(&mut self.session, &input, &res, &self.disc.names, &which, scope) {
            Ok(a) => format!("Added {} population(s) across {} sample(s) ({}). Ctrl+Z undoes it per sample.", a.populations, a.samples, if scope == Scope::FullFile { "whole files" } else { "pooled events only" }),
            Err(e) => format!("Not approved: {e}"),
        };
    }

    pub fn disc_approve_rules(&mut self, scope: Scope) {
        let Some(input) = self.disc.input.clone() else { return };
        let rules = self.rules_core();
        let which = vec![true; rules.len()];
        self.status = match embed::approve_rules(&mut self.session, &input, &rules, &which, &self.disc.thresholds, scope) {
            Ok(a) => format!("Added {} rule population(s) across {} sample(s).", a.populations, a.samples),
            Err(e) => format!("Not approved: {e}"),
        };
    }

    // ---- screens ------------------------------------------------------------------------------------

    pub(super) fn dimred(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples first (File ▸ Load FCS…).").color(theme::MUTED)));
            return;
        }
        self.disc_sync();
        if self.disc.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::SidePanel::left("disc_cfg").resizable(false).exact_width(290.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.disc_config(ui, true));
        });
        egui::SidePanel::right("disc_right").resizable(true).default_width(310.0).width_range(260.0..=460.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                theme::section(ui, "Colour by");
                self.colour_picker(ui);
                self.disc_discovery(ui);
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(10.0))).show_inside(ui, |ui| self.embedding_canvas(ui, ctx));
    }

    fn colour_picker(&mut self, ui: &mut egui::Ui) {
        let feats: Vec<String> = self.disc.input.as_ref().map(|i| i.request.features.clone()).unwrap_or_default();
        let label = match self.disc.color {
            ColorBy::Density => "Density".to_string(),
            ColorBy::Marker(j) => feats.get(j).map_or("Marker".into(), |f| short_name(f)),
            ColorBy::Cluster => "Cluster".into(),
            ColorBy::Sample => "Sample".into(),
            ColorBy::Rule(i) => self.disc.rules.get(i).map_or("Rule".into(), |r| format!("Rule: {}", r.name)),
        };
        let mut next = self.disc.color;
        egui::ComboBox::from_id_salt("disc_color").width(240.0).selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut next, ColorBy::Density, "Density");
            ui.selectable_value(&mut next, ColorBy::Sample, "Sample");
            if self.disc.clus.is_some() {
                ui.selectable_value(&mut next, ColorBy::Cluster, "Cluster");
            }
            for (j, f) in feats.iter().enumerate() {
                ui.selectable_value(&mut next, ColorBy::Marker(j), format!("Marker: {}", short_name(f)));
            }
        });
        if next != self.disc.color {
            self.disc.color = next;
            self.disc.tex = None;
        }
    }

    /// Per-event colours for the embedding.
    fn point_colours(&self, emb: &Embedding) -> Vec<Color32> {
        let input = &emb.input;
        let n = input.n();
        match self.disc.color {
            ColorBy::Density => vec![Color32::from_rgb(0x1F, 0x77, 0xB4); n],
            ColorBy::Sample => input.rows.iter().map(|r| PALETTE[r.0 % PALETTE.len()]).collect(),
            ColorBy::Cluster => match &self.disc.clus {
                Some(c) => c.labels.iter().map(|l| l.map_or(Color32::from_gray(90), |l| PALETTE[l % PALETTE.len()])).collect(),
                None => vec![Color32::GRAY; n],
            },
            ColorBy::Marker(j) => {
                let col = input.column(j);
                let (lo, hi) = (stats::percentile(&col, 1.0), stats::percentile(&col, 99.0));
                let span = if hi > lo { hi - lo } else { 1.0 };
                col.iter().map(|&v| {
                    let [r, g, b] = crate::density::pseudocolor(((v - lo) / span).clamp(0.0, 1.0));
                    Color32::from_rgb(r, g, b)
                }).collect()
            }
            ColorBy::Rule(i) => {
                let rules = self.rules_core();
                match rules.get(i) {
                    Some(r) => {
                        let m = sf_core::discovery::evaluate(&input.feats, n, &input.request.features, r, &self.disc.thresholds);
                        m.iter().map(|&b| if b { Color32::from_rgb(0xD6, 0x27, 0x28) } else { Color32::from_gray(170) }).collect()
                    }
                    None => vec![Color32::GRAY; n],
                }
            }
        }
    }

    fn embedding_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            for tool in [Tool::Pointer, Tool::Polygon, Tool::Rectangle, Tool::Ellipse] {
                if ui.selectable_label(self.disc.tool == tool, tool.label()).clicked() {
                    self.disc.tool = tool;
                    self.disc.drawing.cancel();
                }
            }
            theme::caption(ui, "Draw a region to turn the events inside it into a population.");
            if let Some(l) = self.disc.job_label() {
                ui.add(egui::Spinner::new());
                ui.label(RichText::new(l).color(theme::WARN));
            }
        });
        let Some(emb) = self.disc.emb.clone() else {
            ui.centered_and_justified(|ui| {
                let msg = if self.disc.embed_job.is_some() { "Embedding…" } else { "Choose samples and markers, then press “Run embedding”." };
                ui.label(RichText::new(msg).color(theme::MUTED).size(15.0))
            });
            return;
        };
        let n = emb.input.n();
        if self.disc.range.is_none() {
            let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for i in 0..n {
                let (x, y) = (emb.xy[2 * i], emb.xy[2 * i + 1]);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
            let (px, py) = (0.04 * (x1 - x0).max(1e-9), 0.04 * (y1 - y0).max(1e-9));
            self.disc.range = Some(Range { x0: x0 - px, x1: x1 + px, y0: y0 - py, y1: y1 + py });
        }
        let range = self.disc.range.unwrap();
        let avail = ui.available_rect_before_wrap();
        let plot = Rect::from_min_max(Pos2::new(avail.left() + 8.0, avail.top() + 8.0), Pos2::new(avail.right() - 8.0, avail.bottom() - 8.0));
        if plot.width() < 100.0 || plot.height() < 100.0 {
            return;
        }
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());
        let painter = ui.painter_at(avail);
        painter.rect_filled(plot, 0.0, Color32::WHITE);
        let mut map = Mapper { rect: plot, range };
        self.disc.plot_rect = Some(plot);
        // texture
        let (gw, gh) = (((plot.width() / 1.5) as usize / 16 * 16).clamp(160, 640), ((plot.height() / 1.5) as usize / 16 * 16).clamp(160, 640));
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (n, gw, gh, range.x0.to_bits(), range.x1.to_bits(), range.y0.to_bits(), range.y1.to_bits(), format!("{:?}", self.disc.color), self.disc.clus.as_ref().map(|c| c.n_clusters), self.disc.picked.clone()).hash(&mut h);
            if let ColorBy::Rule(_) = self.disc.color {
                format!("{:?}", self.disc.thresholds).hash(&mut h);
            }
            h.finish()
        };
        if self.disc.tex.is_none() || self.disc.tex_key != key {
            let mut pixels = vec![Color32::TRANSPARENT; gw * gh];
            if self.disc.color == ColorBy::Density {
                let grid = bin((0..n).map(|i| (emb.xy[2 * i], emb.xy[2 * i + 1])), range, gw, gh);
                pixels = render(&grid).into_iter().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
            } else {
                let cols = self.point_colours(&emb);
                let dim = self.disc.color == ColorBy::Cluster && self.disc.picked.iter().any(|&p| p);
                let labels = self.disc.clus.as_ref().map(|c| c.labels.clone());
                for i in 0..n {
                    let (x, y) = (emb.xy[2 * i], emb.xy[2 * i + 1]);
                    if x < range.x0 || x > range.x1 || y < range.y0 || y > range.y1 {
                        continue;
                    }
                    let cx = (((x - range.x0) / (range.x1 - range.x0)) * gw as f64) as usize;
                    let cy = gh - 1 - ((((y - range.y0) / (range.y1 - range.y0)) * gh as f64) as usize).min(gh - 1);
                    let mut c = cols[i];
                    if dim {
                        if let Some(Some(l)) = labels.as_ref().map(|l| l[i]) {
                            if !self.disc.picked.get(l).copied().unwrap_or(false) {
                                c = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 60);
                            }
                        }
                    }
                    for (dx, dy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                        let (px, py) = ((cx + dx).min(gw - 1), (cy + dy).min(gh - 1));
                        pixels[py * gw + px] = c;
                    }
                }
            }
            self.disc.tex = Some(ctx.load_texture("embedding", egui::ColorImage { size: [gw, gh], pixels }, TextureOptions::NEAREST));
            self.disc.tex_key = key;
        }
        if let Some(t) = &self.disc.tex {
            painter.image(t.id(), plot, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        painter.rect_stroke(plot, 0.0, Stroke::new(1.0_f32, Color32::from_gray(150)));
        painter.text(Pos2::new(plot.left() + 6.0, plot.top() + 4.0), Align2::LEFT_TOP, format!("{} · {} events", emb.input.request.method.label(), n), FontId::proportional(11.0), Color32::from_gray(60));
        // zoom / pan
        let hov = resp.hover_pos().filter(|p| plot.contains(*p));
        if let Some(p) = hov {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                let f = (1.0 - scroll as f64 * 0.0015).clamp(0.5, 1.6);
                let c = map.to_data(p);
                if let Some(r) = self.disc.range.as_mut() {
                    r.x0 = c.0 + (r.x0 - c.0) * f;
                    r.x1 = c.0 + (r.x1 - c.0) * f;
                    r.y0 = c.1 + (r.y0 - c.1) * f;
                    r.y1 = c.1 + (r.y1 - c.1) * f;
                    map.range = *r;
                }
            }
            self.disc.drawing.move_to(map.to_data(p));
        }
        if self.disc.tool == Tool::Pointer && resp.dragged() {
            let d = resp.drag_delta();
            let (dx, dy) = ((d.x / plot.width()) as f64 * (range.x1 - range.x0), (d.y / plot.height()) as f64 * (range.y1 - range.y0));
            if let Some(r) = self.disc.range.as_mut() {
                r.x0 -= dx;
                r.x1 -= dx;
                r.y0 += dy;
                r.y1 += dy;
            }
        }
        if resp.double_clicked() && self.disc.tool == Tool::Pointer {
            self.disc.range = None;
        }
        // region drawing -> population
        let pointer = resp.interact_pointer_pos().filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
        let mut finished: Option<Shape> = None;
        match self.disc.tool {
            Tool::Pointer | Tool::Interval | Tool::Quadrant => {}
            Tool::Polygon => {
                if resp.clicked() {
                    let (now, at) = (ctx.input(|i| i.time), resp.interact_pointer_pos());
                    let last = self.disc_last_click;
                    if matches!((last, at), (Some((t, q)), Some(p)) if now - t < 0.4 && q.distance(p) < 6.0) {
                        finished = self.disc.drawing.finish_polygon();
                    } else if let Some(p) = pointer {
                        self.disc.drawing.press(Tool::Polygon, p);
                    }
                    self.disc_last_click = at.map(|p| (now, p));
                }
            }
            tool => {
                if resp.drag_started() {
                    let origin = ctx.input(|i| i.pointer.press_origin()).filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
                    if let Some(p) = origin.or(pointer) {
                        self.disc.drawing.press(tool, p);
                    }
                }
                if resp.drag_stopped() {
                    if let Some(p) = resp.interact_pointer_pos().map(|p| map.to_data(Pos2::new(p.x.clamp(plot.left(), plot.right()), p.y.clamp(plot.top(), plot.bottom())))) {
                        finished = self.disc.drawing.release(tool, p);
                    }
                }
            }
        }
        // in-progress outline
        let pts = self.disc.drawing.points().to_vec();
        let stroke = Stroke::new(1.5_f32, Color32::from_rgb(10, 124, 255));
        match self.disc.tool {
            Tool::Polygon if !pts.is_empty() => {
                let mut s: Vec<Pos2> = pts.iter().map(|&q| map.to_screen(q)).collect();
                if let Some(h) = self.disc.drawing.hover() {
                    s.push(map.to_screen(h));
                }
                painter.add(egui::Shape::line(s, stroke));
            }
            Tool::Rectangle | Tool::Ellipse if pts.len() == 2 => {
                let r = Rect::from_two_pos(map.to_screen(pts[0]), map.to_screen(pts[1]));
                if self.disc.tool == Tool::Rectangle {
                    painter.rect_stroke(r, 0.0, stroke);
                } else {
                    let e: Vec<Pos2> = (0..72).map(|k| { let t = k as f32 * std::f32::consts::TAU / 72.0; r.center() + egui::vec2(r.width() / 2.0 * t.cos(), r.height() / 2.0 * t.sin()) }).collect();
                    painter.add(egui::Shape::closed_line(e, stroke));
                }
            }
            _ => {}
        }
        if let Some(shape) = finished {
            self.disc_region_to_population(&emb, &shape);
        }
    }

    /// Events of the pooled set inside a drawn region become a population in each sample they came from.
    pub fn disc_region_to_population(&mut self, emb: &Embedding, shape: &Shape) {
        let Ok(gate) = to_gate(shape, "x", "y", &sf_core::scale::ScaleSpec::Identity, &sf_core::scale::ScaleSpec::Identity) else { return };
        self.disc.region_count += 1;
        let name = format!("Region {}", self.disc.region_count);
        let mut per: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
        for (i, &(s, r)) in emb.input.rows.iter().enumerate() {
            if gate.contains(emb.xy[2 * i], emb.xy[2 * i + 1]) {
                per.entry(s).or_default().push(r);
            }
        }
        if per.is_empty() {
            self.status = "That region contains no events.".into();
            return;
        }
        let mut total = 0;
        for (s, rows) in per {
            total += rows.len();
            if let Err(e) = self.session.add_index_populations(s, sf_gating::GateTree::ROOT, vec![(name.clone(), rows)]) {
                self.status = e;
                return;
            }
        }
        self.status = format!("'{name}': {total} pooled events added as a population per sample (pooled events only). Ctrl+Z undoes it.");
    }

    pub(super) fn clustering(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples first (File ▸ Load FCS…).").color(theme::MUTED)));
            return;
        }
        self.disc_sync();
        if self.disc.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::SidePanel::left("clus_cfg").resizable(false).exact_width(290.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.disc_config(ui, false));
        });
        egui::SidePanel::right("clus_right").resizable(true).default_width(310.0).width_range(260.0..=460.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.disc_discovery(ui));
        });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            let (Some(input), Some(res)) = (&self.disc.input, &self.disc.clus) else {
                ui.centered_and_justified(|ui| ui.label(RichText::new(if self.disc.cluster_job.is_some() { "Clustering…" } else { "Pool events, then cluster them." }).color(theme::MUTED).size(15.0)));
                return;
            };
            ui.label(RichText::new(format!("Cluster heat map · {} on {} events", res.actual.label(), input.n())).strong());
            theme::caption(ui, "Colour = the cluster's median of each marker, relative to the other clusters (z-score). Numbers are medians in transformed units.");
            ui.add_space(6.0);
            let (k, d) = (res.n_clusters, input.d());
            // per-marker mean / sd of the cluster medians, for the z-scores
            let stat: Vec<(f64, f64)> = (0..d).map(|j| {
                let col: Vec<f64> = (0..k).map(|c| res.medians[c * d + j]).collect();
                (stats::mean(&col), stats::sd(&col))
            }).collect();
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("heat").spacing([3.0, 3.0]).show(ui, |ui| {
                    ui.label("");
                    ui.label("");
                    for f in &input.request.features {
                        ui.label(RichText::new(short_name(f)).size(11.0).color(theme::MUTED));
                    }
                    ui.label(RichText::new("events").size(11.0).color(theme::MUTED));
                    ui.end_row();
                    for c in 0..k {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, PALETTE[c % PALETTE.len()]);
                        ui.label(RichText::new(self.disc.names.get(c).cloned().unwrap_or_default()).size(12.0));
                        for j in 0..d {
                            let v = res.medians[c * d + j];
                            let z = if stat[j].1 > 0.0 { ((v - stat[j].0) / stat[j].1).clamp(-2.0, 2.0) } else { 0.0 };
                            let col = if z >= 0.0 { Color32::from_rgba_unmultiplied(0, 180, 216, (z / 2.0 * 200.0) as u8 + 20) } else { Color32::from_rgba_unmultiplied(214, 39, 40, (-z / 2.0 * 160.0) as u8 + 20) };
                            egui::Frame::none().fill(col).rounding(4.0).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| ui.label(RichText::new(format!("{v:.2}")).monospace().size(11.0)));
                        }
                        ui.label(RichText::new(format!("{}", res.counts[c])).monospace().size(11.0).color(theme::MUTED));
                        ui.end_row();
                    }
                });
            });
        });
    }
}
