//! The StreamFLOW look: the navy/cyan palette of the original app on egui's dark theme, plus the few
//! small widgets the screens share.

use eframe::egui::{self, Color32, Response, RichText, Rounding, Sense, Stroke, Ui, Vec2};

pub const BG: Color32 = Color32::from_rgb(0x0D, 0x1B, 0x2A);
pub const PANEL: Color32 = Color32::from_rgb(0x14, 0x24, 0x35);
pub const PANEL_2: Color32 = Color32::from_rgb(0x1B, 0x2A, 0x3B);
pub const RAISED: Color32 = Color32::from_rgb(0x24, 0x34, 0x47);
pub const INPUT: Color32 = Color32::from_rgb(0x0A, 0x16, 0x22);
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0xB4, 0xD8);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x0B, 0x4A, 0x5E);
pub const TEXT: Color32 = Color32::from_rgb(0xEA, 0xEF, 0xF5);
pub const MUTED: Color32 = Color32::from_rgb(0x8F, 0xA8, 0xC2);
pub const GATE_RED: Color32 = Color32::from_rgb(0xFF, 0x4D, 0x45);
pub const GOOD: Color32 = Color32::from_rgb(0x3F, 0xD0, 0x8A);
pub const WARN: Color32 = Color32::from_rgb(0xF2, 0xB1, 0x34);

/// egui built-in fonts lack many of the symbols the interface uses (▦ ⇄ ✺ ⬠ …). Where the OS ships a symbol
/// font, add it as a FALLBACK so those glyphs render; without it the interface still works, with boxes for them.
fn add_symbol_font(ctx: &egui::Context) {
    let candidates = [r"C:\Windows\Fonts\seguisym.ttf", "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf", "/System/Library/Fonts/Apple Symbols.ttf"];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("symbols".into(), egui::FontData::from_owned(bytes));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push("symbols".into());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

pub fn apply(ctx: &egui::Context) {
    add_symbol_font(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL_2;
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = PANEL;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.window_stroke = Stroke::new(1.0_f32, RAISED);
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);
    let r = Rounding::same(6.0);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = r;
    }
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, RAISED);
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x2E, 0x45, 0x5E);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2E, 0x45, 0x5E);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.open.bg_fill = RAISED;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.window_margin = egui::Margin::same(12.0);
    });
}

/// A pill-shaped toggle button (selected = accent).
pub fn toggle(ui: &mut Ui, selected: bool, text: impl Into<String>) -> Response {
    let text: String = text.into();
    let (fill, color) = if selected { (ACCENT, Color32::from_rgb(0x06, 0x22, 0x2E)) } else { (RAISED, TEXT) };
    ui.add(egui::Button::new(RichText::new(text).color(color)).fill(fill).rounding(Rounding::same(14.0)))
}

/// A small muted caption.
pub fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(11.0).color(MUTED));
}

/// Section title inside a panel.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text.to_uppercase()).size(10.5).strong().color(MUTED));
    ui.add_space(2.0);
}

/// A navigation entry: accent bar + highlighted row when selected; icon only when `expanded` is false.
pub fn nav_item(ui: &mut Ui, icon: &str, label: &str, selected: bool, expanded: bool) -> Response {
    let h = 34.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    let hovered = resp.hovered();
    if selected || hovered {
        let fill = if selected { ACCENT_DIM } else { RAISED };
        ui.painter().rect_filled(rect, Rounding::same(8.0), fill);
    }
    if selected {
        let bar = egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 7.0), Vec2::new(3.0, h - 14.0));
        ui.painter().rect_filled(bar, Rounding::same(2.0), ACCENT);
    }
    let color = if selected { Color32::WHITE } else { TEXT };
    ui.painter().text(egui::pos2(rect.left() + 24.0, rect.center().y), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(16.0), if selected { ACCENT } else { color });
    if expanded {
        ui.painter().text(egui::pos2(rect.left() + 44.0, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), color);
    }
    if !expanded {
        return resp.on_hover_text(label);
    }
    resp
}

/// Horizontal rule in the muted line colour.
pub fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, RAISED);
}
