//! "Something is running" cues, as in the earlier app: a floating chip with a spinner, what is running and for how
//! long, plus a thin animated line along the bottom edge of the window.
//!
//! Work on a background thread (Discover) is reported by its job. Slow work that runs on the interface thread is
//! started through `run_busy`, which shows the cue for two frames *before* the work begins, so it is on screen while
//! the window is busy instead of appearing after the fact.

use super::{theme, App};
use eframe::egui::{self, Align2, Color32, RichText};
use std::time::Instant;

type Work = Box<dyn FnOnce(&mut App)>;

struct Pending {
    label: String,
    work: Work,
    frames: u8,
    since: Instant,
}

#[derive(Default)]
pub struct State {
    pending: Option<Pending>,
    /// What the last frame showed (for tests and the status line).
    pub shown: Option<String>,
}

impl App {
    /// Run `work` on the next-but-one frame with `label` showing as the activity. Only one at a time: a second
    /// request while one is waiting replaces nothing and is ignored (the button that issued it is still busy).
    pub fn run_busy(&mut self, label: impl Into<String>, work: impl FnOnce(&mut App) + 'static) {
        if self.act.pending.is_some() {
            return;
        }
        self.act.pending = Some(Pending { label: label.into(), work: Box::new(work), frames: 0, since: Instant::now() });
    }

    pub fn is_busy(&self) -> bool {
        self.act.pending.is_some() || self.disc.is_busy()
    }

    fn activity_label(&self) -> Option<(String, f32)> {
        if let Some(p) = &self.act.pending {
            return Some((p.label.clone(), p.since.elapsed().as_secs_f32()));
        }
        self.disc.job_label().map(|l| (l, 0.0))
    }

    /// Draw the chip and the bottom line, and start pending work once the cue has been on screen.
    pub(super) fn activity(&mut self, ctx: &egui::Context) {
        let label = self.activity_label();
        self.act.shown = label.as_ref().map(|l| l.0.clone());
        if let Some((text, _)) = &label {
            egui::Area::new(egui::Id::new("activity_chip")).anchor(Align2::LEFT_BOTTOM, [16.0, -46.0]).order(egui::Order::Foreground).interactable(false).show(ctx, |ui| {
                egui::Frame::none().fill(Color32::from_rgba_unmultiplied(20, 30, 45, 235)).rounding(6.0).inner_margin(egui::Margin::symmetric(10.0, 6.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(Color32::from_rgb(0x8F, 0xD3, 0xFF)));
                        ui.label(RichText::new(text).size(12.0).color(Color32::WHITE));
                    });
                });
            });
            // a thin line sweeping along the bottom edge
            let screen = ctx.screen_rect();
            let t = ctx.input(|i| i.time) as f32;
            let phase = (t * 0.8).fract();
            let (w, h) = (screen.width(), 3.0);
            let seg = w * 0.25;
            let x0 = screen.left() - seg + phase * (w + seg);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("activity_line")));
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - h), egui::pos2(screen.right(), screen.bottom())), 0.0, Color32::from_rgba_unmultiplied(0x2D, 0x6C, 0xDF, 50));
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x0.max(screen.left()), screen.bottom() - h), egui::pos2((x0 + seg).min(screen.right()), screen.bottom())), 0.0, theme::ACCENT);
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        // let two frames paint the cue, then do the work
        let go = self.act.pending.as_mut().map_or(false, |p| {
            p.frames += 1;
            p.frames > 2
        });
        if go {
            if let Some(p) = self.act.pending.take() {
                (p.work)(self);
            }
        }
    }
}
