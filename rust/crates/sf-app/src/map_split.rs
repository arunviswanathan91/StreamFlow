//! A pooled map (t-SNE / UMAP) shown as one panel per group or per sample, on shared axes and with the SAME number
//! of events in every panel, so that a denser region in one panel means a real difference and not a bigger file.
//! Pure functions: which events go in which panel, a panel as pixels, and the whole comparison as a figure.

use crate::density::{bin, render, Range};
use crate::embed::EmbedInput;
use crate::figure::{Align, Figure, Item, Stroke, Text, VAlign};
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitBy {
    Off,
    Group,
    Sample,
}

impl SplitBy {
    pub const ALL: [SplitBy; 3] = [SplitBy::Off, SplitBy::Group, SplitBy::Sample];
    pub fn label(self) -> &'static str {
        match self {
            SplitBy::Off => "One map",
            SplitBy::Group => "Split by group",
            SplitBy::Sample => "Split by sample",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapPanel {
    pub title: String,
    /// Indices into the pooled events, already thinned to the common panel size.
    pub events: Vec<usize>,
    /// How many pooled events this group or sample has in all.
    pub available: usize,
}

/// The panels of a split map. By group: one per sample group that has pooled events, plus "Not in a group" for
/// pooled samples that belong to none (a sample in two groups is in both panels). By sample: one per pooled
/// sample. Every panel is thinned, evenly and without randomness, to the size of the smallest.
pub fn split_panels(session: &Session, input: &EmbedInput, by: SplitBy) -> Vec<MapPanel> {
    let of_samples = |samples: &[usize]| -> Vec<usize> { (0..input.n()).filter(|&i| samples.contains(&input.rows[i].0)).collect() };
    let mut raw: Vec<(String, Vec<usize>)> = match by {
        SplitBy::Off => vec![],
        SplitBy::Sample => input.request.samples.iter().map(|&si| (session.sample(si).name.trim_end_matches(".fcs").to_string(), of_samples(&[si]))).collect(),
        SplitBy::Group => {
            let mut out: Vec<(String, Vec<usize>)> = Vec::new();
            let mut grouped: Vec<usize> = Vec::new();
            for g in &session.groups {
                let members = session.member_indices(&g.name);
                grouped.extend(&members);
                out.push((g.name.clone(), of_samples(&members)));
            }
            let rest: Vec<usize> = input.request.samples.iter().copied().filter(|s| !grouped.contains(s)).collect();
            if !out.is_empty() && !rest.is_empty() {
                out.push(("Not in a group".to_string(), of_samples(&rest)));
            }
            out
        }
    };
    raw.retain(|(_, e)| !e.is_empty());
    let Some(n_min) = raw.iter().map(|(_, e)| e.len()).min() else { return vec![] };
    raw.into_iter()
        .map(|(title, e)| {
            let len = e.len();
            MapPanel { title, events: (0..n_min).map(|k| e[k * len / n_min]).collect(), available: len }
        })
        .collect()
}

/// One panel as `w x h` RGB pixels on white, row 0 at the top. `colours` gives every pooled event its colour;
/// `None` draws the density of the panel's events instead.
pub fn panel_rgb(xy: &[f64], events: &[usize], colours: Option<&[[u8; 3]]>, range: Range, w: usize, h: usize) -> Vec<u8> {
    let mut rgb = vec![255u8; w * h * 3];
    match colours {
        None => {
            let grid = bin(events.iter().map(|&i| (xy[2 * i], xy[2 * i + 1])), range, w, h);
            for (k, p) in render(&grid).iter().enumerate().take(w * h) {
                let a = p[3] as f32 / 255.0;
                for c in 0..3 {
                    rgb[k * 3 + c] = (p[c] as f32 * a + 255.0 * (1.0 - a)).round() as u8;
                }
            }
        }
        Some(cols) => {
            for &i in events {
                let (x, y) = (xy[2 * i], xy[2 * i + 1]);
                if !(x.is_finite() && y.is_finite()) || x < range.x0 || x > range.x1 || y < range.y0 || y > range.y1 {
                    continue;
                }
                let cx = ((((x - range.x0) / (range.x1 - range.x0)) * w as f64) as usize).min(w - 1);
                let cy = h - 1 - ((((y - range.y0) / (range.y1 - range.y0)) * h as f64) as usize).min(h - 1);
                for (dx, dy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                    let k = ((cy + dy).min(h - 1) * w + (cx + dx).min(w - 1)) * 3;
                    rgb[k..k + 3].copy_from_slice(&cols[i]);
                }
            }
        }
    }
    rgb
}

/// The comparison as a figure for PDF / SVG / PNG: a title, then the panels in rows of `cols`, each with its name
/// and event count. `images[k]` is panel `k` as `px x px` RGB.
pub fn split_figure(title: &str, note: &str, panels: &[MapPanel], images: &[Vec<u8>], px: usize, cols: usize) -> Figure {
    let (side, gap, head, label) = (190.0_f32, 14.0_f32, 44.0_f32, 16.0_f32);
    let cols = cols.clamp(1, panels.len().max(1));
    let rows = panels.len().div_ceil(cols);
    let mut fig = Figure::new(gap + cols as f32 * (side + gap), head + rows as f32 * (side + label + gap));
    let ink = [40, 40, 40, 255];
    fig.push(Item::Text(Text::new(gap, 18.0, title, 11.0).align(Align::Left, VAlign::Middle).bold().color(ink)));
    fig.push(Item::Text(Text::new(gap, 33.0, note, 7.5).align(Align::Left, VAlign::Middle).color([110, 110, 110, 255])));
    for (k, (p, img)) in panels.iter().zip(images).enumerate() {
        let (x, y) = (gap + (k % cols) as f32 * (side + gap), head + (k / cols) as f32 * (side + label + gap));
        fig.push(Item::Text(Text::new(x, y + label / 2.0, format!("{} ({} events)", p.title, p.events.len()), 8.0).align(Align::Left, VAlign::Middle).color(ink)));
        fig.push(Item::Image { x, y: y + label, w: side, h: side, px_w: px as u32, px_h: px as u32, rgb: img.clone() });
        fig.push(Item::Rect { x, y: y + label, w: side, h: side, fill: None, stroke: Some(Stroke::new([150, 150, 150, 255], 0.6)) });
    }
    fig
}
