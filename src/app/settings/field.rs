use eframe::egui;

// ── Field kinds ────────────────────────────────────────────────────────────

/// A single editable numeric field.
pub(super) struct NumField {
    pub label: &'static str,
    pub value: f32,
    pub default: f32,
    pub step: f32,
    pub min: f32,
    pub max: f32,
    pub unit: &'static str,
}

impl NumField {
    pub fn nudge(&mut self, delta: f32) {
        self.value = (self.value + delta * self.step).clamp(self.min, self.max);
    }
    pub fn reset(&mut self) { self.value = self.default; }
}

/// A discrete toggle field (cycles through a fixed list of string labels).
pub(super) struct ToggleField {
    pub label: &'static str,
    pub options: &'static [&'static str],
    pub index: usize,
    pub default: usize,
}

impl ToggleField {
    pub fn next(&mut self) { self.index = (self.index + 1) % self.options.len(); }
    pub fn prev(&mut self) { self.index = (self.index + self.options.len() - 1) % self.options.len(); }
    pub fn reset(&mut self) { self.index = self.default; }
    pub fn value_str(&self) -> &'static str { self.options[self.index] }
}

/// A text-edit field (e.g. file path or PSK31 message).
pub(super) struct TextField {
    pub label: &'static str,
    pub value: String,
    /// Default value restored on R-reset. Empty string for WAV path.
    pub default_value: String,
    /// None = not yet tried; Some(true) = last load ok; Some(false) = last load failed.
    pub status: Option<bool>,
}

impl TextField {
    pub fn reset(&mut self) { self.value = self.default_value.clone(); self.status = None; }
}

// ── Row enum — unifies the three field kinds ───────────────────────────────

pub(super) enum Row {
    Num(NumField),
    Toggle(ToggleField),
    Text(TextField),
}

impl Row {
    pub fn label(&self) -> &str {
        match self {
            Row::Num(f) => f.label,
            Row::Toggle(f) => f.label,
            Row::Text(f) => f.label,
        }
    }
    pub fn nudge_right(&mut self) {
        match self {
            Row::Num(f) => f.nudge(1.0),
            Row::Toggle(f) => f.next(),
            Row::Text(_) => {}
        }
    }
    pub fn nudge_left(&mut self) {
        match self {
            Row::Num(f) => f.nudge(-1.0),
            Row::Toggle(f) => f.prev(),
            Row::Text(_) => {}
        }
    }
    pub fn reset(&mut self) {
        match self {
            Row::Num(f) => f.reset(),
            Row::Toggle(f) => f.reset(),
            Row::Text(f) => f.reset(),
        }
    }
    /// Patch a numeric row: clamp `v` to [min, max] and set both value and default.
    pub fn patch_num(&mut self, v: f32) {
        if let Row::Num(f) = self {
            let clamped = v.clamp(f.min, f.max);
            f.value   = clamped;
            f.default = clamped;
        }
    }
}

// ── Drawing helpers ────────────────────────────────────────────────────────

/// Draw a numeric row value.
pub(super) fn draw_num(
    painter: &egui::Painter,
    f: &NumField,
    x: f32, y: f32, row_h: f32,
    rect_right: f32,
    med: &egui::FontId,
    small: &egui::FontId,
    val_color: egui::Color32,
    focused: bool,
) {
    let val_str = if f.step < 0.1 {
        format!("{:.2}{}", f.value, f.unit)
    } else if f.step < 1.0 {
        format!("{:.1}{}", f.value, f.unit)
    } else {
        format!("{:.0}{}", f.value, f.unit)
    };
    painter.text(
        egui::pos2(x, y + row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        val_str,
        med.clone(),
        val_color,
    );
    if focused {
        painter.text(
            egui::pos2(rect_right - 14.0, y + row_h / 2.0),
            egui::Align2::RIGHT_CENTER,
            "◀ ▶",
            small.clone(),
            egui::Color32::from_gray(140),
        );
    }
}

/// Draw a toggle row value.
pub(super) fn draw_toggle(
    painter: &egui::Painter,
    f: &ToggleField,
    x: f32, y: f32, row_h: f32,
    med: &egui::FontId,
    val_color: egui::Color32,
    focused: bool,
) {
    let val_str = format!("◀ {} ▶", f.value_str());
    painter.text(
        egui::pos2(x, y + row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        val_str,
        med.clone(),
        if focused { egui::Color32::WHITE } else { val_color },
    );
}
