use eframe::egui::{
    self, Color32, CornerRadius, FontId, Response, RichText, Stroke, TextStyle, UiBuilder, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(4, 5, 6);
pub const PANEL: Color32 = Color32::from_rgb(9, 11, 12);
pub const SURFACE: Color32 = Color32::from_rgb(9, 11, 12);
pub const TEXT: Color32 = Color32::from_rgb(212, 216, 220);
pub const MUTED: Color32 = Color32::from_rgb(104, 111, 118);
pub const ACCENT: Color32 = Color32::from_rgb(126, 217, 160);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(17, 21, 23);
pub const BORDER: Color32 = Color32::from_rgb(26, 31, 35);
pub const WARN: Color32 = Color32::from_rgb(240, 113, 113);
pub const AMBER: Color32 = Color32::from_rgb(214, 179, 74);

pub fn is_mobile() -> bool {
    cfg!(target_os = "ios")
}

pub fn page_pad() -> i8 {
    14
}

pub fn row_height() -> f32 {
    if is_mobile() {
        30.0
    } else {
        26.0
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.faint_bg_color = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(6, 8, 6);
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = ACCENT_DIM;
    visuals.widgets.hovered.weak_bg_fill = ACCENT_DIM;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_fill = ACCENT_DIM;
    visuals.widgets.active.weak_bg_fill = ACCENT_DIM;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.inactive.corner_radius = CornerRadius::ZERO;
    visuals.widgets.hovered.corner_radius = CornerRadius::ZERO;
    visuals.widgets.active.corner_radius = CornerRadius::ZERO;
    visuals.window_corner_radius = CornerRadius::ZERO;
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(6.0, 6.0);
    style.spacing.button_padding = Vec2::new(6.0, 3.0);
    style.spacing.interact_size.y = if is_mobile() { 34.0 } else { 24.0 };
    style.text_styles = [
        (TextStyle::Heading, FontId::monospace(15.0)),
        (TextStyle::Body, FontId::monospace(12.5)),
        (TextStyle::Button, FontId::monospace(12.5)),
        (TextStyle::Small, FontId::monospace(10.5)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    ctx.set_style(style);
}

pub fn truncated(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    size: f32,
    color: Color32,
    width: f32,
) {
    ui.add_sized(
        Vec2::new(width.max(24.0), 16.0),
        egui::Label::new(
            RichText::new(text.into())
                .monospace()
                .size(size)
                .color(color),
        )
        .truncate(),
    );
}

pub fn mono(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text.into()).monospace().size(size)
}

pub fn section_header(ui: &mut egui::Ui, title: &str, meta: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("-- {}", title.to_lowercase()))
                .monospace()
                .size(10.5)
                .color(MUTED),
        );
        if let Some(meta) = meta {
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                Vec2::new(width, 16.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.label(RichText::new(meta).monospace().size(10.5).color(MUTED));
                },
            );
        }
    });
}

/// Compact bordered button for secondary chrome (dialogs, headers).
pub fn compact_button(ui: &mut egui::Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("[ {label} ]"))
                .monospace()
                .size(9.5)
                .color(MUTED),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(3))
        .min_size(Vec2::new(0.0, if is_mobile() { 26.0 } else { 20.0 })),
    )
}

/// Compact accented button for the primary action of a dialog.
pub fn compact_primary(ui: &mut egui::Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("[ {label} ]"))
                .monospace()
                .size(9.5)
                .color(ACCENT),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, ACCENT))
        .corner_radius(CornerRadius::same(3))
        .min_size(Vec2::new(0.0, if is_mobile() { 28.0 } else { 22.0 })),
    )
}

/// Section header with a compact action button on the right. Returns true when clicked.
pub fn section_header_button(
    ui: &mut egui::Ui,
    title: &str,
    meta: Option<&str>,
    action: &str,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("-- {}", title.to_lowercase()))
                .monospace()
                .size(10.5)
                .color(MUTED),
        );
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            Vec2::new(width, 20.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                clicked = compact_button(ui, action).clicked();
                if let Some(meta) = meta {
                    ui.label(RichText::new(meta).monospace().size(10.5).color(MUTED));
                }
            },
        );
    });
    clicked
}

pub fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{key:<7}"))
                .monospace()
                .size(11.0)
                .color(MUTED),
        );
        let width = (ui.available_width() - 2.0).max(40.0);
        truncated(ui, value, 12.0, TEXT, width);
    });
}

pub fn progress_bar(fraction: f32, width: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0)) * width as f32).round() as usize;
    format!("[{}{}]", "█".repeat(filled), "·".repeat(width - filled))
}

pub fn action_button(ui: &mut egui::Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("[ {label} ]"))
                .monospace()
                .color(ACCENT),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::ZERO)
        .min_size(Vec2::new(0.0, if is_mobile() { 34.0 } else { 24.0 })),
    )
}

pub fn muted_button(ui: &mut egui::Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("[ {label} ]"))
                .monospace()
                .color(MUTED),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::ZERO)
        .min_size(Vec2::new(0.0, if is_mobile() { 34.0 } else { 24.0 })),
    )
}

pub fn row(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    add: impl FnOnce(&mut egui::Ui),
) -> Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_height()), egui::Sense::hover());
    let response = ui.interact(rect, id, egui::Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, ACCENT_DIM);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
            0.0,
            ACCENT,
        );
    }
    let inner = rect.shrink2(Vec2::new(6.0, 2.0));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        add,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

pub fn toast(ctx: &egui::Context, text: &str, is_error: bool) {
    let color = if is_error { WARN } else { ACCENT };
    let prefix = if is_error { "!" } else { "+" };
    egui::Area::new(egui::Id::new("droplink-toast"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::Vec2::new(12.0, -10.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("{prefix} {text}"))
                            .monospace()
                            .size(11.0)
                            .color(color),
                    );
                });
        });
}
