use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const BG: Color32 = Color32::from_rgb(15, 17, 21);
pub const PANEL: Color32 = Color32::from_rgb(24, 27, 34);
pub const ACCENT: Color32 = Color32::from_rgb(59, 130, 246);
pub const TEXT: Color32 = Color32::from_rgb(228, 231, 236);
pub const MUTED: Color32 = Color32::from_rgb(140, 148, 160);
pub const OK: Color32 = Color32::from_rgb(52, 211, 153);
pub const WARN: Color32 = Color32::from_rgb(248, 113, 113);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.faint_bg_color = Color32::from_rgb(30, 34, 42);
    visuals.extreme_bg_color = Color32::from_rgb(10, 12, 16);
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT;
    visuals.hyperlink_color = ACCENT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(40, 45, 55));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 38, 47);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(34, 38, 47);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 54, 66);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.window_corner_radius = CornerRadius::same(14);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    ctx.set_style(style);
}
