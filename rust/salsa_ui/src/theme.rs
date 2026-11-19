use std::collections::BTreeSet;

use salsa_tools::ipc::{
    active_theme_name_for_tray, load_gui_settings, load_saved_theme_name, update_gui_settings,
};
use salsa_tools::json::JsonValue;

pub const CUSTOM_THEME_NAME: &str = "Custom";
pub const UI_FONT_SCALE_DEFAULT_PCT: i32 = 100;
pub const UI_FONT_SCALE_MIN_PCT: i32 = 75;
pub const UI_FONT_SCALE_MAX_PCT: i32 = 160;
pub const UI_FONT_SCALE_STEP_PCT: i32 = 5;

pub fn normalize_font_scale_pct(pct: i32) -> i32 {
    pct.clamp(UI_FONT_SCALE_MIN_PCT, UI_FONT_SCALE_MAX_PCT)
}

pub fn apply_font_scale_step(cur_pct: i32, delta_or_zero: i32) -> i32 {
    if delta_or_zero == 0 {
        UI_FONT_SCALE_DEFAULT_PCT
    } else {
        normalize_font_scale_pct(cur_pct + delta_or_zero)
    }
}

pub fn font_scale_factor_from_pct(pct: i32) -> f64 {
    f64::from(normalize_font_scale_pct(pct)) / 100.0
}

pub fn extract_font_scale_pct_from_json(settings: &JsonValue) -> i32 {
    let raw = settings.get("ui_font_scale_pct").and_then(|v| {
        v.as_i64()
            .map(|n| n as i32)
            .or_else(|| v.as_f64().map(|f| f.round() as i32))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i32>().ok()))
    });
    normalize_font_scale_pct(raw.unwrap_or(UI_FONT_SCALE_DEFAULT_PCT))
}

pub fn load_ui_font_scale_pct() -> i32 {
    extract_font_scale_pct_from_json(&load_gui_settings())
}

pub fn save_ui_font_scale_pct(pct: i32) -> i32 {
    let norm = normalize_font_scale_pct(pct);
    let val = JsonValue::Int(i64::from(norm));
    let _ = update_gui_settings(&[
        ("ui_font_scale_pct", val.clone()),
        ("font_scale_pct", val),
    ]);
    norm
}

pub const THEME_COLOR_KEYS: &[&str] = &[
    "bg_main",
    "bg_card",
    "bg_deep",
    "bg_canvas",
    "bg_control",
    "bg_hover",
    "border_card",
    "border_control",
    "text_primary",
    "text_bright",
    "text_secondary",
    "text_muted",
    "text_dim",
    "accent_primary",
    "accent_secondary",
    "accent_hot",
    "accent_warn",
    "accent_mic",
    "accent_midi",
    "accent_out",
    "btn_primary_bg",
    "btn_primary_border",
    "btn_primary_hover",
    "btn_primary_text",
    "btn_accent_bg",
    "btn_accent_border",
    "btn_accent_hover",
    "btn_accent_text",
    "combo_popup_bg",
    "combo_popup_sel_bg",
    "combo_popup_sel_text",
    "canvas_grid",
    "spectrum_grid",
    "grid_zero",
    "node_bg_top",
    "node_bg_hover",
    "node_bg_bot",
    "node_border",
    "wire_pulse",
    "handle_border",
    "code_bg",
];

pub type ThemeTokenEntry = (&'static str, &'static str, &'static str);
pub type ThemeTokenGroup = (&'static str, &'static [ThemeTokenEntry]);

pub const THEME_TOKEN_GROUPS: &[ThemeTokenGroup] = &[
    (
        "Surfaces",
        &[
            ("bg_main", "Main Window BG", "Outer window & tab canvas background"),
            ("bg_card", "Card Surface", "Header, EQ card, mixer & session panels"),
            ("bg_deep", "Deep Well / Saved Pill BG", "Tables, saved pills & log console"),
            ("bg_canvas", "Spectrum & Graph Canvas", "EQ spectrum & Auto-Graph backdrop"),
            ("bg_control", "Control / Button / Slider", "Combo boxes, buttons & slider grooves"),
            ("bg_hover", "Control Hover", "Hover highlight on buttons & dropdowns"),
            ("border_card", "Card Border", "Outer panel border"),
            ("border_control", "Control Border", "Inputs, saved pills & control outlines"),
            ("code_bg", "Manual Code Highlight", "Inline code background in Owner's Manual"),
        ],
    ),
    (
        "Accents",
        &[
            ("accent_primary", "Primary Accent", "EQ curve, active badges & saved pills"),
            ("accent_secondary", "Secondary Accent", "Sub-banner, sliders & peak meter"),
            ("accent_hot", "Warm / Peak Accent", "Spectrum bar tops & EQ card accent"),
            ("accent_warn", "Dynamic Delta Accent", "Dotted dynamic EQ delta & Δ labels"),
            ("accent_mic", "Mic / Capture Accent", "Capture meters & error alerts"),
            ("accent_midi", "MIDI / Synth Accent", "MIDI hub graph nodes & traces"),
            ("accent_out", "Output / Sink Accent", "Final hardware sink stage accent"),
            ("handle_border", "Handle Rim Color", "Outline around EQ dots & slider knobs"),
            ("wire_pulse", "Signal Pulse Dot", "Moving topology animation dot"),
        ],
    ),
    (
        "Text / Btns",
        &[
            ("text_bright", "Bright Headings", "Studio banner & control values"),
            ("text_primary", "Primary Text", "Standard body labels & table entries"),
            ("text_secondary", "Secondary Info", "Device endpoints & manual prose"),
            ("text_muted", "Muted Labels", "Frequency ticks & inactive tab text"),
            ("text_dim", "Dim Labels", "Spectrum dB scale markers"),
            ("btn_primary_bg", "Primary Button BG", "Engine toggle & Benchmark button"),
            ("btn_primary_border", "Primary Button Border", "Outline for primary buttons"),
            ("btn_primary_hover", "Primary Button Hover", "Hover fill for primary buttons"),
            ("btn_primary_text", "Primary Button Text", "Label color on primary buttons"),
            ("btn_accent_bg", "Accent Action BG", "Alert / action accent button fill"),
            ("btn_accent_border", "Accent Action Border", "Outline for accent buttons"),
            ("btn_accent_hover", "Accent Action Hover", "Hover fill for accent buttons"),
            ("btn_accent_text", "Accent Action Text", "Label color on accent buttons"),
        ],
    ),
    (
        "Popups / Grids",
        &[
            ("combo_popup_bg", "Dropdown Popup BG", "Background of open combo lists"),
            ("combo_popup_sel_bg", "Dropdown Selected BG", "Highlighted row inside combo popup"),
            ("combo_popup_sel_text", "Dropdown Selected Text", "Text color on selected combo row"),
            ("canvas_grid", "Auto-Graph Grid Lines", "Subtle background grid in topology view"),
            ("spectrum_grid", "Spectrum dB Grid", "EQ spectrum grid lines"),
            ("grid_zero", "Spectrum 0 dB Line", "Center 0 dB reference line"),
            ("node_bg_top", "Topology Node Top", "Top gradient stop on graph boxes"),
            ("node_bg_hover", "Topology Node Hover", "Selected/hover node fill"),
            ("node_bg_bot", "Topology Node Bottom", "Bottom gradient stop on graph boxes"),
            ("node_border", "Topology Node Border", "Outline of Auto-Graph nodes"),
        ],
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalsaTheme {
    pub name: String,
    pub is_light: bool,
    pub bg_main: String,
    pub bg_card: String,
    pub bg_deep: String,
    pub bg_canvas: String,
    pub bg_control: String,
    pub bg_hover: String,
    pub border_card: String,
    pub border_control: String,
    pub text_primary: String,
    pub text_bright: String,
    pub text_secondary: String,
    pub text_muted: String,
    pub text_dim: String,
    pub accent_primary: String,
    pub accent_secondary: String,
    pub accent_warn: String,
    pub accent_mic: String,
    pub accent_hot: String,
    pub accent_midi: String,
    pub accent_out: String,
    pub btn_primary_bg: String,
    pub btn_primary_border: String,
    pub btn_primary_hover: String,
    pub btn_primary_text: String,
    pub btn_accent_bg: String,
    pub btn_accent_border: String,
    pub btn_accent_hover: String,
    pub btn_accent_text: String,
    pub combo_popup_bg: String,
    pub combo_popup_sel_bg: String,
    pub combo_popup_sel_text: String,
    pub canvas_grid: String,
    pub spectrum_grid: String,
    pub grid_zero: String,
    pub node_bg_top: String,
    pub node_bg_hover: String,
    pub node_bg_bot: String,
    pub node_border: String,
    pub wire_pulse: String,
    pub handle_border: String,
    pub code_bg: String,
}

#[derive(Clone, Copy)]
struct PresetSpec {
    name: &'static str,
    is_light: bool,
    bg_main: &'static str,
    bg_card: &'static str,
    bg_deep: &'static str,
    bg_canvas: &'static str,
    bg_control: &'static str,
    bg_hover: &'static str,
    border_card: &'static str,
    border_control: &'static str,
    text_primary: &'static str,
    text_bright: &'static str,
    text_secondary: &'static str,
    text_muted: &'static str,
    text_dim: &'static str,
    accent_primary: &'static str,
    accent_secondary: &'static str,
    accent_warn: &'static str,
    accent_mic: &'static str,
    accent_hot: &'static str,
    accent_midi: &'static str,
    accent_out: &'static str,
    btn_primary_bg: &'static str,
    btn_primary_border: &'static str,
    btn_primary_hover: &'static str,
    btn_primary_text: &'static str,
    btn_accent_bg: &'static str,
    btn_accent_border: &'static str,
    btn_accent_hover: &'static str,
    btn_accent_text: &'static str,
    combo_popup_bg: &'static str,
    combo_popup_sel_bg: &'static str,
    combo_popup_sel_text: &'static str,
    canvas_grid: &'static str,
    spectrum_grid: &'static str,
    grid_zero: &'static str,
    node_bg_top: &'static str,
    node_bg_hover: &'static str,
    node_bg_bot: &'static str,
    node_border: &'static str,
    wire_pulse: &'static str,
    handle_border: &'static str,
    code_bg: &'static str,
}

impl From<&PresetSpec> for SalsaTheme {
    fn from(s: &PresetSpec) -> Self {
        Self {
            name: s.name.to_string(),
            is_light: s.is_light,
            bg_main: s.bg_main.to_string(),
            bg_card: s.bg_card.to_string(),
            bg_deep: s.bg_deep.to_string(),
            bg_canvas: s.bg_canvas.to_string(),
            bg_control: s.bg_control.to_string(),
            bg_hover: s.bg_hover.to_string(),
            border_card: s.border_card.to_string(),
            border_control: s.border_control.to_string(),
            text_primary: s.text_primary.to_string(),
            text_bright: s.text_bright.to_string(),
            text_secondary: s.text_secondary.to_string(),
            text_muted: s.text_muted.to_string(),
            text_dim: s.text_dim.to_string(),
            accent_primary: s.accent_primary.to_string(),
            accent_secondary: s.accent_secondary.to_string(),
            accent_warn: s.accent_warn.to_string(),
            accent_mic: s.accent_mic.to_string(),
            accent_hot: s.accent_hot.to_string(),
            accent_midi: s.accent_midi.to_string(),
            accent_out: s.accent_out.to_string(),
            btn_primary_bg: s.btn_primary_bg.to_string(),
            btn_primary_border: s.btn_primary_border.to_string(),
            btn_primary_hover: s.btn_primary_hover.to_string(),
            btn_primary_text: s.btn_primary_text.to_string(),
            btn_accent_bg: s.btn_accent_bg.to_string(),
            btn_accent_border: s.btn_accent_border.to_string(),
            btn_accent_hover: s.btn_accent_hover.to_string(),
            btn_accent_text: s.btn_accent_text.to_string(),
            combo_popup_bg: s.combo_popup_bg.to_string(),
            combo_popup_sel_bg: s.combo_popup_sel_bg.to_string(),
            combo_popup_sel_text: s.combo_popup_sel_text.to_string(),
            canvas_grid: s.canvas_grid.to_string(),
            spectrum_grid: s.spectrum_grid.to_string(),
            grid_zero: s.grid_zero.to_string(),
            node_bg_top: s.node_bg_top.to_string(),
            node_bg_hover: s.node_bg_hover.to_string(),
            node_bg_bot: s.node_bg_bot.to_string(),
            node_border: s.node_border.to_string(),
            wire_pulse: s.wire_pulse.to_string(),
            handle_border: s.handle_border.to_string(),
            code_bg: s.code_bg.to_string(),
        }
    }
}

impl SalsaTheme {
    pub fn get_token<'a>(&'a self, key: &str) -> &'a str {
        match key {
            "bg_main" => &self.bg_main,
            "bg_card" => &self.bg_card,
            "bg_deep" => &self.bg_deep,
            "bg_canvas" => &self.bg_canvas,
            "bg_control" => &self.bg_control,
            "bg_hover" => &self.bg_hover,
            "border_card" => &self.border_card,
            "border_control" => &self.border_control,
            "text_primary" => &self.text_primary,
            "text_bright" => &self.text_bright,
            "text_secondary" => &self.text_secondary,
            "text_muted" => &self.text_muted,
            "text_dim" => &self.text_dim,
            "accent_primary" => &self.accent_primary,
            "accent_secondary" => &self.accent_secondary,
            "accent_warn" => &self.accent_warn,
            "accent_mic" => &self.accent_mic,
            "accent_hot" => &self.accent_hot,
            "accent_midi" => &self.accent_midi,
            "accent_out" => &self.accent_out,
            "btn_primary_bg" => &self.btn_primary_bg,
            "btn_primary_border" => &self.btn_primary_border,
            "btn_primary_hover" => &self.btn_primary_hover,
            "btn_primary_text" => &self.btn_primary_text,
            "btn_accent_bg" => &self.btn_accent_bg,
            "btn_accent_border" => &self.btn_accent_border,
            "btn_accent_hover" => &self.btn_accent_hover,
            "btn_accent_text" => &self.btn_accent_text,
            "combo_popup_bg" => &self.combo_popup_bg,
            "combo_popup_sel_bg" => &self.combo_popup_sel_bg,
            "combo_popup_sel_text" => &self.combo_popup_sel_text,
            "canvas_grid" => &self.canvas_grid,
            "spectrum_grid" => &self.spectrum_grid,
            "grid_zero" => &self.grid_zero,
            "node_bg_top" => &self.node_bg_top,
            "node_bg_hover" => &self.node_bg_hover,
            "node_bg_bot" => &self.node_bg_bot,
            "node_border" => &self.node_border,
            "wire_pulse" => &self.wire_pulse,
            "handle_border" => &self.handle_border,
            "code_bg" => &self.code_bg,
            _ => &self.accent_primary,
        }
    }

    pub fn set_token(&mut self, key: &str, hex: &str) {
        let cur = self.get_token(key).to_string();
        let clean = normalize_hex_str(hex, &cur);
        match key {
            "bg_main" => self.bg_main = clean,
            "bg_card" => self.bg_card = clean,
            "bg_deep" => self.bg_deep = clean,
            "bg_canvas" => self.bg_canvas = clean,
            "bg_control" => self.bg_control = clean,
            "bg_hover" => self.bg_hover = clean,
            "border_card" => self.border_card = clean,
            "border_control" => self.border_control = clean,
            "text_primary" => self.text_primary = clean,
            "text_bright" => self.text_bright = clean,
            "text_secondary" => self.text_secondary = clean,
            "text_muted" => self.text_muted = clean,
            "text_dim" => self.text_dim = clean,
            "accent_primary" => self.accent_primary = clean,
            "accent_secondary" => self.accent_secondary = clean,
            "accent_warn" => self.accent_warn = clean,
            "accent_mic" => self.accent_mic = clean,
            "accent_hot" => self.accent_hot = clean,
            "accent_midi" => self.accent_midi = clean,
            "accent_out" => self.accent_out = clean,
            "btn_primary_bg" => self.btn_primary_bg = clean,
            "btn_primary_border" => self.btn_primary_border = clean,
            "btn_primary_hover" => self.btn_primary_hover = clean,
            "btn_primary_text" => self.btn_primary_text = clean,
            "btn_accent_bg" => self.btn_accent_bg = clean,
            "btn_accent_border" => self.btn_accent_border = clean,
            "btn_accent_hover" => self.btn_accent_hover = clean,
            "btn_accent_text" => self.btn_accent_text = clean,
            "combo_popup_bg" => self.combo_popup_bg = clean,
            "combo_popup_sel_bg" => self.combo_popup_sel_bg = clean,
            "combo_popup_sel_text" => self.combo_popup_sel_text = clean,
            "canvas_grid" => self.canvas_grid = clean,
            "spectrum_grid" => self.spectrum_grid = clean,
            "grid_zero" => self.grid_zero = clean,
            "node_bg_top" => self.node_bg_top = clean,
            "node_bg_hover" => self.node_bg_hover = clean,
            "node_bg_bot" => self.node_bg_bot = clean,
            "node_border" => self.node_border = clean,
            "wire_pulse" => self.wire_pulse = clean,
            "handle_border" => self.handle_border = clean,
            "code_bg" => self.code_bg = clean,
            _ => {}
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut entries = Vec::with_capacity(THEME_COLOR_KEYS.len() + 1);
        entries.push(("is_light", JsonValue::Bool(self.is_light)));
        for &key in THEME_COLOR_KEYS {
            entries.push((key, JsonValue::Str(self.get_token(key).to_string())));
        }
        JsonValue::obj(entries)
    }

    pub fn from_json(raw: &JsonValue, fallback: &SalsaTheme) -> Self {
        let mut out = fallback.clone();
        out.is_light = raw
            .get("is_light")
            .and_then(JsonValue::as_bool)
            .unwrap_or(fallback.is_light);
        for &key in THEME_COLOR_KEYS {
            let v = valid_hex(raw.get(key), fallback.get_token(key));
            out.set_token(key, &v);
        }
        out
    }
}

const PRESET_SPECS: &[PresetSpec] = &[
    PresetSpec {
        name: "Salsa Midnight (Default)",
        is_light: false,
        bg_main: "#090d16",
        bg_card: "#111827",
        bg_deep: "#0b1120",
        bg_canvas: "#070b14",
        bg_control: "#1e293b",
        bg_hover: "#334155",
        border_card: "#1f2937",
        border_control: "#334155",
        text_primary: "#e2e8f0",
        text_bright: "#f8fafc",
        text_secondary: "#cbd5e1",
        text_muted: "#94a3b8",
        text_dim: "#64748b",
        accent_primary: "#00e676",
        accent_secondary: "#38bdf8",
        accent_warn: "#f59e0b",
        accent_mic: "#f43f5e",
        accent_hot: "#ff5e3a",
        accent_midi: "#a855f7",
        accent_out: "#10b981",
        btn_primary_bg: "#059669",
        btn_primary_border: "#10b981",
        btn_primary_hover: "#10b981",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#e11d48",
        btn_accent_border: "#f43f5e",
        btn_accent_hover: "#f43f5e",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#111827",
        combo_popup_sel_bg: "#1e293b",
        combo_popup_sel_text: "#00e676",
        canvas_grid: "#0f172a",
        spectrum_grid: "#162032",
        grid_zero: "#334155",
        node_bg_top: "#111827",
        node_bg_hover: "#162033",
        node_bg_bot: "#0b1120",
        node_border: "#334155",
        wire_pulse: "#ffffff",
        handle_border: "#ffffff",
        code_bg: "#1e293b",
    },
    PresetSpec {
        name: "Plasma Breeze Emerald",
        is_light: false,
        bg_main: "#161b1f",
        bg_card: "#1e2429",
        bg_deep: "#181d22",
        bg_canvas: "#14191d",
        bg_control: "#252c33",
        bg_hover: "#2f3841",
        border_card: "#2c343c",
        border_control: "#36404a",
        text_primary: "#e6edf3",
        text_bright: "#f8fafc",
        text_secondary: "#cbd5e1",
        text_muted: "#9ca3af",
        text_dim: "#6b7280",
        accent_primary: "#14b866",
        accent_secondary: "#38bdf8",
        accent_warn: "#f59e0b",
        accent_mic: "#f43f5e",
        accent_hot: "#ff5e3a",
        accent_midi: "#a855f7",
        accent_out: "#10b981",
        btn_primary_bg: "#10b981",
        btn_primary_border: "#14b866",
        btn_primary_hover: "#14b866",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#e11d48",
        btn_accent_border: "#f43f5e",
        btn_accent_hover: "#f43f5e",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#1e2429",
        combo_popup_sel_bg: "#252c33",
        combo_popup_sel_text: "#14b866",
        canvas_grid: "#1b2127",
        spectrum_grid: "#20272e",
        grid_zero: "#36404a",
        node_bg_top: "#1e2429",
        node_bg_hover: "#252c33",
        node_bg_bot: "#171c20",
        node_border: "#36404a",
        wire_pulse: "#ffffff",
        handle_border: "#ffffff",
        code_bg: "#252c33",
    },
    PresetSpec {
        name: "CachyOS Teal Void",
        is_light: false,
        bg_main: "#0a0e14",
        bg_card: "#111722",
        bg_deep: "#080b10",
        bg_canvas: "#070a0f",
        bg_control: "#192231",
        bg_hover: "#243247",
        border_card: "#1c2738",
        border_control: "#2b3c54",
        text_primary: "#e2f1f8",
        text_bright: "#f8fafc",
        text_secondary: "#bfd7e6",
        text_muted: "#85a1b8",
        text_dim: "#5b7388",
        accent_primary: "#00d4aa",
        accent_secondary: "#22d3ee",
        accent_warn: "#fbbf24",
        accent_mic: "#fb7185",
        accent_hot: "#ff6b4a",
        accent_midi: "#818cf8",
        accent_out: "#14b8a6",
        btn_primary_bg: "#0d9488",
        btn_primary_border: "#14b8a6",
        btn_primary_hover: "#14b8a6",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#e11d48",
        btn_accent_border: "#fb7185",
        btn_accent_hover: "#f43f5e",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#111722",
        combo_popup_sel_bg: "#192231",
        combo_popup_sel_text: "#00d4aa",
        canvas_grid: "#111c29",
        spectrum_grid: "#142233",
        grid_zero: "#2b3c54",
        node_bg_top: "#111722",
        node_bg_hover: "#172233",
        node_bg_bot: "#090d14",
        node_border: "#2b3c54",
        wire_pulse: "#ffffff",
        handle_border: "#ffffff",
        code_bg: "#192231",
    },
    PresetSpec {
        name: "Nordic Frost (Nord)",
        is_light: false,
        bg_main: "#242933",
        bg_card: "#2e3440",
        bg_deep: "#1f242d",
        bg_canvas: "#1b2028",
        bg_control: "#3b4252",
        bg_hover: "#4c566a",
        border_card: "#3b4252",
        border_control: "#4c566a",
        text_primary: "#e5e9f0",
        text_bright: "#eceff4",
        text_secondary: "#d8dee9",
        text_muted: "#9aa5ba",
        text_dim: "#6f7b91",
        accent_primary: "#a3be8c",
        accent_secondary: "#88c0d0",
        accent_warn: "#ebcb8b",
        accent_mic: "#bf616a",
        accent_hot: "#d08770",
        accent_midi: "#b48ead",
        accent_out: "#8fbcbb",
        btn_primary_bg: "#4b7a62",
        btn_primary_border: "#a3be8c",
        btn_primary_hover: "#5c9377",
        btn_primary_text: "#eceff4",
        btn_accent_bg: "#a5424c",
        btn_accent_border: "#bf616a",
        btn_accent_hover: "#bf616a",
        btn_accent_text: "#eceff4",
        combo_popup_bg: "#2e3440",
        combo_popup_sel_bg: "#434c5e",
        combo_popup_sel_text: "#88c0d0",
        canvas_grid: "#262c37",
        spectrum_grid: "#282f3b",
        grid_zero: "#4c566a",
        node_bg_top: "#2e3440",
        node_bg_hover: "#3b4252",
        node_bg_bot: "#242933",
        node_border: "#4c566a",
        wire_pulse: "#eceff4",
        handle_border: "#eceff4",
        code_bg: "#3b4252",
    },
    PresetSpec {
        name: "Catppuccin Mocha",
        is_light: false,
        bg_main: "#181825",
        bg_card: "#1e1e2e",
        bg_deep: "#11111b",
        bg_canvas: "#11111b",
        bg_control: "#313244",
        bg_hover: "#45475a",
        border_card: "#313244",
        border_control: "#45475a",
        text_primary: "#cdd6f4",
        text_bright: "#f5e0dc",
        text_secondary: "#bac2de",
        text_muted: "#9399b2",
        text_dim: "#6c7086",
        accent_primary: "#a6e3a1",
        accent_secondary: "#89b4fa",
        accent_warn: "#f9e2af",
        accent_mic: "#f38ba8",
        accent_hot: "#fab387",
        accent_midi: "#cba6f7",
        accent_out: "#94e2d5",
        btn_primary_bg: "#2f7d57",
        btn_primary_border: "#a6e3a1",
        btn_primary_hover: "#3b986b",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#be3b5e",
        btn_accent_border: "#f38ba8",
        btn_accent_hover: "#d6486e",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#1e1e2e",
        combo_popup_sel_bg: "#313244",
        combo_popup_sel_text: "#cba6f7",
        canvas_grid: "#1c1c2c",
        spectrum_grid: "#1e1e2e",
        grid_zero: "#45475a",
        node_bg_top: "#1e1e2e",
        node_bg_hover: "#2a2b3d",
        node_bg_bot: "#141420",
        node_border: "#45475a",
        wire_pulse: "#cdd6f4",
        handle_border: "#cdd6f4",
        code_bg: "#313244",
    },
    PresetSpec {
        name: "Dracula Console",
        is_light: false,
        bg_main: "#21222c",
        bg_card: "#282a36",
        bg_deep: "#191a21",
        bg_canvas: "#17181f",
        bg_control: "#383b4d",
        bg_hover: "#44475a",
        border_card: "#383b4d",
        border_control: "#6272a4",
        text_primary: "#f8f8f2",
        text_bright: "#ffffff",
        text_secondary: "#d6d8e5",
        text_muted: "#9ea8cc",
        text_dim: "#6272a4",
        accent_primary: "#50fa7b",
        accent_secondary: "#8be9fd",
        accent_warn: "#f1fa8c",
        accent_mic: "#ff5555",
        accent_hot: "#ffb86c",
        accent_midi: "#bd93f9",
        accent_out: "#50fa7b",
        btn_primary_bg: "#2ea043",
        btn_primary_border: "#50fa7b",
        btn_primary_hover: "#3fb950",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#d6336c",
        btn_accent_border: "#ff79c6",
        btn_accent_hover: "#e64980",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#282a36",
        combo_popup_sel_bg: "#44475a",
        combo_popup_sel_text: "#50fa7b",
        canvas_grid: "#242633",
        spectrum_grid: "#262835",
        grid_zero: "#44475a",
        node_bg_top: "#282a36",
        node_bg_hover: "#343746",
        node_bg_bot: "#1e2029",
        node_border: "#44475a",
        wire_pulse: "#f8f8f2",
        handle_border: "#f8f8f2",
        code_bg: "#383b4d",
    },
    PresetSpec {
        name: "Synthwave Outrun",
        is_light: false,
        bg_main: "#0d0618",
        bg_card: "#160b29",
        bg_deep: "#080311",
        bg_canvas: "#07020f",
        bg_control: "#241342",
        bg_hover: "#361d61",
        border_card: "#2e1854",
        border_control: "#5b21b6",
        text_primary: "#f3e8ff",
        text_bright: "#ffffff",
        text_secondary: "#d8b4fe",
        text_muted: "#a78bfa",
        text_dim: "#7c3aed",
        accent_primary: "#00f5d4",
        accent_secondary: "#00bbf9",
        accent_warn: "#fee440",
        accent_mic: "#fb7185",
        accent_hot: "#ff2a85",
        accent_midi: "#c084fc",
        accent_out: "#00f5d4",
        btn_primary_bg: "#009688",
        btn_primary_border: "#00f5d4",
        btn_primary_hover: "#00bfa5",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#db2777",
        btn_accent_border: "#ff2a85",
        btn_accent_hover: "#ec4899",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#160b29",
        combo_popup_sel_bg: "#2e1854",
        combo_popup_sel_text: "#00f5d4",
        canvas_grid: "#1a0d30",
        spectrum_grid: "#20103a",
        grid_zero: "#4c1d95",
        node_bg_top: "#180c2c",
        node_bg_hover: "#251345",
        node_bg_bot: "#0c0518",
        node_border: "#4c1d95",
        wire_pulse: "#ffffff",
        handle_border: "#ffffff",
        code_bg: "#241342",
    },
    PresetSpec {
        name: "Analog Studio Rack",
        is_light: false,
        bg_main: "#131110",
        bg_card: "#1d1916",
        bg_deep: "#0d0b0a",
        bg_canvas: "#0b0908",
        bg_control: "#2b2521",
        bg_hover: "#3c342e",
        border_card: "#302924",
        border_control: "#4a3f37",
        text_primary: "#f5ebe0",
        text_bright: "#fff8f0",
        text_secondary: "#d6ccc2",
        text_muted: "#a89f96",
        text_dim: "#736960",
        accent_primary: "#f59e0b",
        accent_secondary: "#38bdf8",
        accent_warn: "#fbbf24",
        accent_mic: "#fb7185",
        accent_hot: "#ef4444",
        accent_midi: "#c084fc",
        accent_out: "#22c55e",
        btn_primary_bg: "#b45309",
        btn_primary_border: "#f59e0b",
        btn_primary_hover: "#d97706",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#b91c1c",
        btn_accent_border: "#ef4444",
        btn_accent_hover: "#dc2626",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#1d1916",
        combo_popup_sel_bg: "#3c342e",
        combo_popup_sel_text: "#f59e0b",
        canvas_grid: "#1a1612",
        spectrum_grid: "#201b16",
        grid_zero: "#4a3f37",
        node_bg_top: "#1f1a17",
        node_bg_hover: "#2d2621",
        node_bg_bot: "#120f0d",
        node_border: "#4a3f37",
        wire_pulse: "#fff8f0",
        handle_border: "#fff8f0",
        code_bg: "#2b2521",
    },
    PresetSpec {
        name: "Daylight Mastering (Light)",
        is_light: true,
        bg_main: "#eef2f6",
        bg_card: "#ffffff",
        bg_deep: "#f8fafc",
        bg_canvas: "#f8fafc",
        bg_control: "#e2e8f0",
        bg_hover: "#cbd5e1",
        border_card: "#cbd5e1",
        border_control: "#94a3b8",
        text_primary: "#0f172a",
        text_bright: "#020617",
        text_secondary: "#334155",
        text_muted: "#475569",
        text_dim: "#64748b",
        accent_primary: "#047857",
        accent_secondary: "#0369a1",
        accent_warn: "#b45309",
        accent_mic: "#be123c",
        accent_hot: "#ea580c",
        accent_midi: "#7e22ce",
        accent_out: "#059669",
        btn_primary_bg: "#059669",
        btn_primary_border: "#047857",
        btn_primary_hover: "#047857",
        btn_primary_text: "#ffffff",
        btn_accent_bg: "#e11d48",
        btn_accent_border: "#be123c",
        btn_accent_hover: "#be123c",
        btn_accent_text: "#ffffff",
        combo_popup_bg: "#ffffff",
        combo_popup_sel_bg: "#dbeafe",
        combo_popup_sel_text: "#0369a1",
        canvas_grid: "#e2e8f0",
        spectrum_grid: "#e2e8f0",
        grid_zero: "#94a3b8",
        node_bg_top: "#ffffff",
        node_bg_hover: "#f1f5f9",
        node_bg_bot: "#e2e8f0",
        node_border: "#94a3b8",
        wire_pulse: "#0f172a",
        handle_border: "#0f172a",
        code_bg: "#e2e8f0",
    },
];

pub fn preset_theme_names() -> Vec<&'static str> {
    PRESET_SPECS.iter().map(|s| s.name).collect()
}

pub fn normalize_hex_str(s: &str, default: &str) -> String {
    let t = s.trim();
    if t.is_ascii()
        && t.len() == 7
        && t.starts_with('#')
        && t[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        t.to_ascii_lowercase()
    } else {
        default.to_ascii_lowercase()
    }
}

fn valid_hex(raw: Option<&JsonValue>, default: &str) -> String {
    let Some(s) = raw.and_then(JsonValue::as_str) else {
        return default.to_ascii_lowercase();
    };
    normalize_hex_str(s, default)
}

pub fn mix_hex(hex_a: &str, hex_b: &str, t: f64) -> String {
    let (ra, ga, ba) = parse_hex_rgb(hex_a);
    let (rb, gb, bb) = parse_hex_rgb(hex_b);
    let f = t.clamp(0.0, 1.0);
    let r = ((ra * (1.0 - f) + rb * f) * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = ((ga * (1.0 - f) + gb * f) * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = ((ba * (1.0 - f) + bb * f) * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn derive_harmony_theme(base: &SalsaTheme) -> SalsaTheme {
    let mut out = base.clone();
    let bg = out.bg_main.clone();
    let pri = out.accent_primary.clone();
    let sec = out.accent_secondary.clone();
    let hot = out.accent_hot.clone();

    if out.is_light {
        let white = "#ffffff";
        let ink = "#0f172a";
        out.bg_card = mix_hex(&bg, white, 0.72);
        out.bg_deep = mix_hex(&bg, white, 0.55);
        out.bg_canvas = mix_hex(&bg, white, 0.65);
        out.bg_control = mix_hex(&bg, ink, 0.08);
        out.bg_hover = mix_hex(&bg, ink, 0.16);
        out.border_card = mix_hex(&bg, ink, 0.18);
        out.border_control = mix_hex(&bg, &pri, 0.35);
        out.text_bright = "#020617".to_string();
        out.text_primary = "#0f172a".to_string();
        out.text_secondary = "#334155".to_string();
        out.text_muted = "#475569".to_string();
        out.text_dim = "#64748b".to_string();
        out.combo_popup_bg = out.bg_card.clone();
        out.combo_popup_sel_bg = mix_hex(&out.bg_card, &pri, 0.20);
        out.combo_popup_sel_text = mix_hex(&pri, ink, 0.30);
        out.canvas_grid = mix_hex(&out.bg_canvas, ink, 0.10);
        out.spectrum_grid = mix_hex(&out.bg_canvas, ink, 0.12);
        out.grid_zero = mix_hex(&out.bg_canvas, ink, 0.35);
        out.node_bg_top = out.bg_card.clone();
        out.node_bg_hover = mix_hex(&out.bg_card, &sec, 0.12);
        out.node_bg_bot = mix_hex(&bg, ink, 0.05);
        out.node_border = out.border_control.clone();
        out.wire_pulse = "#0f172a".to_string();
        out.handle_border = "#0f172a".to_string();
        out.code_bg = out.bg_control.clone();
    } else {
        let black = "#030408";
        let white = "#f8fafc";
        out.bg_card = mix_hex(&bg, white, 0.06);
        out.bg_deep = mix_hex(&bg, black, 0.32);
        out.bg_canvas = mix_hex(&bg, black, 0.42);
        let ctl0 = mix_hex(&bg, &pri, 0.10);
        out.bg_control = mix_hex(&ctl0, white, 0.08);
        out.bg_hover = mix_hex(&out.bg_control, white, 0.12);
        out.border_card = mix_hex(&bg, white, 0.14);
        out.border_control = mix_hex(&out.bg_control, &pri, 0.32);
        out.text_bright = "#f8fafc".to_string();
        out.text_primary = mix_hex("#e2e8f0", &sec, 0.08);
        out.text_secondary = mix_hex("#cbd5e1", &sec, 0.14);
        out.text_muted = mix_hex("#94a3b8", &bg, 0.15);
        out.text_dim = mix_hex("#64748b", &pri, 0.15);
        out.combo_popup_bg = out.bg_card.clone();
        out.combo_popup_sel_bg = mix_hex(&out.bg_control, &pri, 0.25);
        out.combo_popup_sel_text = pri.clone();
        out.canvas_grid = mix_hex(&out.bg_canvas, white, 0.06);
        out.spectrum_grid = mix_hex(&out.bg_canvas, white, 0.09);
        out.grid_zero = mix_hex(&out.bg_canvas, &pri, 0.28);
        out.node_bg_top = mix_hex(&out.bg_card, white, 0.04);
        out.node_bg_hover = mix_hex(&out.bg_card, &sec, 0.14);
        out.node_bg_bot = out.bg_deep.clone();
        out.node_border = out.border_control.clone();
        out.wire_pulse = "#ffffff".to_string();
        out.handle_border = "#ffffff".to_string();
        out.code_bg = out.bg_control.clone();
    }

    out.btn_primary_bg = mix_hex(&pri, "#0f172a", 0.36);
    out.btn_primary_border = pri;
    out.btn_primary_hover = mix_hex(&out.btn_primary_border, "#0f172a", 0.16);
    out.btn_primary_text = "#ffffff".to_string();
    out.btn_accent_bg = mix_hex(&hot, "#0f172a", 0.28);
    out.btn_accent_border = hot.clone();
    out.btn_accent_hover = mix_hex(&hot, "#ffffff", 0.12);
    out.btn_accent_text = "#ffffff".to_string();
    out
}

pub fn theme_by_name(name: &str) -> SalsaTheme {
    if name == CUSTOM_THEME_NAME || name == "Custom Studio Theme" {
        let mut base = SalsaTheme::from(&PRESET_SPECS[0]);
        base.name = name.to_string();
        let settings = load_gui_settings();
        if let Some(raw) = settings.get("custom_theme") {
            let mut loaded = SalsaTheme::from_json(raw, &base);
            loaded.name = name.to_string();
            return loaded;
        }
        return base;
    }

    if let Some(spec) = PRESET_SPECS.iter().find(|s| s.name == name) {
        return SalsaTheme::from(spec);
    }
    SalsaTheme::from(&PRESET_SPECS[0])
}

pub fn load_studio_theme() -> SalsaTheme {
    theme_by_name(&load_saved_theme_name())
}

pub fn load_tray_theme() -> SalsaTheme {
    theme_by_name(&active_theme_name_for_tray())
}

pub fn parse_hex_rgb(hex: &str) -> (f64, f64, f64) {
    let t = hex.trim().trim_start_matches('#');
    if t.is_ascii() && t.len() >= 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&t[0..2], 16),
            u8::from_str_radix(&t[2..4], 16),
            u8::from_str_radix(&t[4..6], 16),
        ) {
            return (
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
            );
        }
    }
    (0.0, 0.9, 0.46)
}

fn hex_rgba_css(hex: &str, alpha: f64) -> String {
    let (r, g, b) = parse_hex_rgb(hex);
    format!(
        "rgba({}, {}, {}, {:.2})",
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
        alpha.clamp(0.0, 1.0)
    )
}

pub fn build_gtk4_css(t: &SalsaTheme, compact: bool) -> String {
    let pad_v = if compact { 3 } else { 5 };
    let pad_h = if compact { 9 } else { 12 };
    let font_sz = if compact { 11 } else { 13 };
    let title_sz = if compact { 13 } else { 18 };
    let sub_sz = if compact { 9 } else { 11 };
    let sec_sz = if compact { 10 } else { 13 };
    let val_sz = if compact { 9 } else { 11 };
    let dyn_sz = if compact { 8 } else { 10 };
    let acc_tint = hex_rgba_css(&t.accent_primary, 0.14);
    let acc_border_tint = hex_rgba_css(&t.accent_primary, 0.44);
    let sec_tint = hex_rgba_css(&t.accent_secondary, 0.12);
    let sec_border_tint = hex_rgba_css(&t.accent_secondary, 0.49);
    let err_tint = hex_rgba_css(&t.accent_mic, 0.16);

    format!(
        "window, .salsa-win, .salsa-dbg-root {{\n\
         background-color: {bg_main};\n\
         color: {text_primary};\n\
         font-family: 'Inter', 'Noto Sans', sans-serif;\n\
         font-size: {font_sz}px;\n\
         }}\n\
         .salsa-transparent-win {{\n\
         background-color: transparent;\n\
         background-image: none;\n\
         }}\n\
         button, togglebutton, checkbutton, entry, spinbutton, dropdown, dropdown > button, popover, notebook, scrollbar, scrollbar slider, scale, scale slider, progressbar {{\n\
         background-image: none;\n\
         box-shadow: none;\n\
         text-shadow: none;\n\
         outline: none;\n\
         }}\n\
         scrollbar {{\n\
         background-color: transparent;\n\
         border: none;\n\
         margin: 0px;\n\
         padding: 0px;\n\
         min-width: 8px;\n\
         min-height: 8px;\n\
         }}\n\
         scrollbar trough {{\n\
         background-color: {bg_deep};\n\
         border: none;\n\
         border-radius: 4px;\n\
         min-width: 8px;\n\
         min-height: 8px;\n\
         }}\n\
         scrollbar slider {{\n\
         background-color: {border_control};\n\
         border: none;\n\
         border-radius: 4px;\n\
         margin: 1px;\n\
         min-width: 6px;\n\
         min-height: 6px;\n\
         }}\n\
         scrollbar slider:hover {{\n\
         background-color: {text_muted};\n\
         }}\n\
         .salsa-mixer-card {{\n\
         background-color: {bg_card};\n\
         border: 1px solid {border_control};\n\
         border-top-left-radius: 10px;\n\
         border-top-right-radius: 10px;\n\
         border-bottom-left-radius: 0px;\n\
         border-bottom-right-radius: 0px;\n\
         padding: 12px 16px;\n\
         }}\n\
         .salsa-menu-card {{\n\
         background-color: {bg_card};\n\
         border: 1px solid {border_control};\n\
         border-radius: 10px;\n\
         padding: 8px;\n\
         }}\n\
         .salsa-card, .salsa-dbg-card, frame.salsa-card {{\n\
         background-color: {bg_card};\n\
         border: 1px solid {border_card};\n\
         border-radius: 9px;\n\
         padding: 8px 11px;\n\
         }}\n\
         .salsa-subcard {{\n\
         background-color: {bg_deep};\n\
         border: 1px solid {border_card};\n\
         border-radius: 10px;\n\
         padding: 8px 10px;\n\
         }}\n\
         .salsa-bt-pill {{\n\
         background-color: {sec_tint};\n\
         border: 1px solid {sec_border_tint};\n\
         border-radius: 11px;\n\
         padding: 6px 10px;\n\
         }}\n\
         label {{\n\
         color: {text_primary};\n\
         background-color: transparent;\n\
         }}\n\
         .salsa-title-banner {{\n\
         color: {text_bright};\n\
         font-size: {title_sz}px;\n\
         font-weight: 800;\n\
         }}\n\
         .salsa-sub-banner {{\n\
         color: {accent_secondary};\n\
         font-size: {sub_sz}px;\n\
         font-weight: 600;\n\
         }}\n\
         .salsa-ver-pill {{\n\
         background-color: {bg_deep};\n\
         color: {accent_warn};\n\
         border: 1px solid {accent_warn};\n\
         border-radius: 5px;\n\
         padding: 1px 6px;\n\
         font-size: 11px;\n\
         font-weight: 800;\n\
         }}\n\
         .salsa-studio-hdr {{\n\
         color: {accent_primary};\n\
         font-size: {sec_sz}px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-saved-chip {{\n\
         background-color: {bg_deep};\n\
         color: {accent_primary};\n\
         border: 1px solid {border_control};\n\
         border-radius: 6px;\n\
         padding: 3px 7px;\n\
         font-size: {val_sz}px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-band-val {{\n\
         color: {text_bright};\n\
         font-size: {val_sz}px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-band-dyn {{\n\
         color: {accent_warn};\n\
         font-family: monospace;\n\
         font-size: {dyn_sz}px;\n\
         }}\n\
         .salsa-band-freq {{\n\
         color: {text_muted};\n\
         font-size: {val_sz}px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-peaks-lbl {{\n\
         color: {accent_secondary};\n\
         font-family: monospace;\n\
         font-size: {val_sz}px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-auto-badge {{\n\
         color: {accent_secondary};\n\
         font-size: {val_sz}px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-manual-box {{\n\
         background-color: {bg_deep};\n\
         border: 1px solid {border_control};\n\
         border-radius: 8px;\n\
         padding: 10px 14px;\n\
         }}\n\
         .salsa-bright {{\n\
         color: {text_bright};\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-tray-title {{\n\
         color: {text_bright};\n\
         font-size: 15px;\n\
         font-weight: 800;\n\
         }}\n\
         .salsa-dim {{\n\
         color: {text_muted};\n\
         font-size: 11px;\n\
         font-weight: 600;\n\
         }}\n\
         .salsa-drag-grip {{\n\
         color: {text_muted};\n\
         font-size: 13px;\n\
         padding: 0px 2px 0px 0px;\n\
         }}\n\
         .salsa-section-title {{\n\
         color: {text_muted};\n\
         font-size: 11px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-section-cap {{\n\
         color: {text_muted};\n\
         font-size: 10px;\n\
         font-weight: 700;\n\
         letter-spacing: 0.8px;\n\
         }}\n\
         .salsa-status-pill {{\n\
         background-color: {acc_tint};\n\
         color: {accent_primary};\n\
         border: 1px solid {accent_primary};\n\
         border-radius: 8px;\n\
         padding: 2px 7px;\n\
         font-size: 10px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-status-offline {{\n\
         background-color: {err_tint};\n\
         color: {accent_mic};\n\
         border: 1px solid {accent_mic};\n\
         border-radius: 8px;\n\
         padding: 2px 7px;\n\
         font-size: 10px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-hw-badge {{\n\
         background-color: {acc_tint};\n\
         color: {accent_primary};\n\
         border: 1px solid {acc_border_tint};\n\
         border-radius: 9px;\n\
         padding: 3px 8px;\n\
         font-size: 11px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-dbg-header {{\n\
         color: {accent_primary};\n\
         font-size: 12px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-dbg-stat {{\n\
         color: {text_secondary};\n\
         font-family: monospace;\n\
         font-size: 11px;\n\
         }}\n\
         .salsa-sc-viewport {{\n\
         background-color: {bg_canvas};\n\
         border: 1px solid {border_control};\n\
         border-radius: 8px;\n\
         }}\n\
         .salsa-table-hdr {{\n\
         background-color: {bg_control};\n\
         color: {text_bright};\n\
         border-bottom: 1px solid {border_control};\n\
         padding: 4px 8px;\n\
         font-size: 11px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-accent {{\n\
         color: {accent_primary};\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-warn-lbl {{\n\
         color: {accent_warn};\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-err-lbl {{\n\
         color: {accent_mic};\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-sec-lbl {{\n\
         color: {accent_secondary};\n\
         font-weight: 700;\n\
         }}\n\
         button, togglebutton {{\n\
         background-color: {bg_control};\n\
         color: {text_bright};\n\
         border: 1px solid {border_control};\n\
         border-radius: 6px;\n\
         padding: {pad_v}px {pad_h}px;\n\
         min-height: 20px;\n\
         font-size: 11px;\n\
         font-weight: 600;\n\
         }}\n\
         button.salsa-btn-compact, togglebutton.salsa-btn-compact {{\n\
         padding: 3px 8px;\n\
         min-height: 18px;\n\
         font-size: {val_sz}px;\n\
         }}\n\
         button.salsa-key-btn {{\n\
         background-color: {bg_deep};\n\
         color: {text_bright};\n\
         border: 1px solid {border_control};\n\
         border-radius: 6px;\n\
         padding: 3px 8px;\n\
         font-family: monospace;\n\
         font-weight: 700;\n\
         }}\n\
         button.salsa-key-btn:checked {{\n\
         background-color: {bg_hover};\n\
         border-color: {accent_primary};\n\
         color: {accent_primary};\n\
         }}\n\
         button:hover, togglebutton:hover {{\n\
         background-color: {bg_hover};\n\
         border-color: {accent_secondary};\n\
         }}\n\
         button.salsa-primary, button.salsa-btn-primary {{\n\
         background-color: {btn_primary_bg};\n\
         color: {btn_primary_text};\n\
         border: 1px solid {btn_primary_border};\n\
         font-weight: 700;\n\
         }}\n\
         button.salsa-primary:hover, button.salsa-btn-primary:hover {{\n\
         background-color: {btn_primary_hover};\n\
         }}\n\
         button.salsa-btn-accent {{\n\
         background-color: {btn_accent_bg};\n\
         color: {btn_accent_text};\n\
         border: 1px solid {btn_accent_border};\n\
         font-weight: 700;\n\
         }}\n\
         button.salsa-btn-accent:hover {{\n\
         background-color: {btn_accent_hover};\n\
         }}\n\
         button.salsa-warn, button.salsa-btn-warn {{\n\
         background-color: {bg_control};\n\
         color: {accent_warn};\n\
         border: 1px solid {border_control};\n\
         font-weight: 600;\n\
         }}\n\
         button.salsa-warn:hover, button.salsa-btn-warn:hover {{\n\
         background-color: {bg_hover};\n\
         border-color: {accent_warn};\n\
         }}\n\
         button.salsa-hdr-btn, button.salsa-hdr-close {{\n\
         background-color: {bg_control};\n\
         color: {text_muted};\n\
         border: 1px solid {border_control};\n\
         border-radius: 7px;\n\
         padding: 2px 6px;\n\
         min-height: 18px;\n\
         font-size: 10px;\n\
         font-weight: 700;\n\
         }}\n\
         button.salsa-hdr-btn:hover, button.salsa-hdr-close:hover {{\n\
         background-color: {bg_hover};\n\
         color: {text_bright};\n\
         border-color: {accent_primary};\n\
         }}\n\
         button.salsa-hdr-btn:checked {{\n\
         background-color: {bg_hover};\n\
         border-color: {accent_primary};\n\
         color: {accent_primary};\n\
         }}\n\
         button.salsa-toggle:checked, togglebutton.salsa-toggle:checked {{\n\
         background-color: {bg_hover};\n\
         color: {accent_primary};\n\
         border-color: {accent_primary};\n\
         font-weight: 700;\n\
         }}\n\
         button.salsa-mute-on, togglebutton.salsa-mute-on:checked {{\n\
         background-color: {accent_mic};\n\
         color: #ffffff;\n\
         border-color: {accent_mic};\n\
         font-weight: 700;\n\
         }}\n\
         button.salsa-tab, button.salsa-tab-btn {{\n\
         background-color: transparent;\n\
         border: none;\n\
         border-bottom: 2px solid transparent;\n\
         border-radius: 0px;\n\
         color: {text_muted};\n\
         font-size: 12px;\n\
         font-weight: 600;\n\
         padding: 5px 12px 7px 12px;\n\
         }}\n\
         button.salsa-tab:hover, button.salsa-tab-btn:hover {{\n\
         background-color: transparent;\n\
         color: {text_primary};\n\
         }}\n\
         button.salsa-tab:checked, button.salsa-tab-btn:checked, button.salsa-tab-active {{\n\
         background-color: transparent;\n\
         color: {text_bright};\n\
         border-bottom: 2px solid {accent_primary};\n\
         }}\n\
         button.salsa-step-pill {{\n\
         background-color: {bg_deep};\n\
         border: 1px solid {border_control};\n\
         border-radius: 5px;\n\
         color: {accent_primary};\n\
         font-size: 11px;\n\
         font-weight: 700;\n\
         padding: 2px 7px;\n\
         min-height: 18px;\n\
         }}\n\
         button.salsa-step-pill:hover {{\n\
         background-color: {bg_hover};\n\
         border-color: {accent_primary};\n\
         }}\n\
         button.salsa-menu-item {{\n\
         background-color: transparent;\n\
         border: 1px solid transparent;\n\
         border-radius: 6px;\n\
         color: {text_primary};\n\
         padding: 6px 12px;\n\
         font-size: 12px;\n\
         font-weight: 600;\n\
         }}\n\
         button.salsa-menu-item:hover {{\n\
         background-color: {bg_hover};\n\
         border-color: {border_control};\n\
         color: {text_bright};\n\
         }}\n\
         entry, spinbutton {{\n\
         background-color: {bg_deep};\n\
         color: {text_bright};\n\
         border: 1px solid {border_control};\n\
         border-radius: 6px;\n\
         padding: 2px 6px;\n\
         min-height: 22px;\n\
         }}\n\
         entry:focus-within, spinbutton:focus-within {{\n\
         border-color: {accent_primary};\n\
         }}\n\
         scale trough {{\n\
         background-color: {bg_control};\n\
         border: 1px solid {border_card};\n\
         border-radius: 4px;\n\
         min-width: 6px;\n\
         min-height: 6px;\n\
         }}\n\
         scale.horizontal highlight {{\n\
         background-color: {accent_secondary};\n\
         border-radius: 4px;\n\
         min-height: 6px;\n\
         }}\n\
         scale.vertical highlight {{\n\
         background-color: {accent_primary};\n\
         border-radius: 4px;\n\
         min-width: 6px;\n\
         }}\n\
         scale.horizontal slider {{\n\
         background-color: {accent_secondary};\n\
         border: 2px solid {handle_border};\n\
         border-radius: 6px;\n\
         min-width: 12px;\n\
         min-height: 12px;\n\
         margin: 0px;\n\
         }}\n\
         scale.vertical slider {{\n\
         background-color: {accent_primary};\n\
         border: 2px solid {handle_border};\n\
         border-radius: 6px;\n\
         min-width: 12px;\n\
         min-height: 12px;\n\
         margin: 0px;\n\
         }}\n\
         dropdown > button {{\n\
         background-color: {bg_control};\n\
         color: {text_bright};\n\
         border: 1px solid {border_control};\n\
         border-radius: 6px;\n\
         padding: 3px 8px;\n\
         min-height: 20px;\n\
         font-size: 11px;\n\
         font-weight: 600;\n\
         }}\n\
         dropdown > button:hover {{\n\
         background-color: {bg_hover};\n\
         border-color: {accent_secondary};\n\
         }}\n\
         popover > contents {{\n\
         background-color: {combo_popup_bg};\n\
         color: {text_bright};\n\
         border: 1px solid {border_control};\n\
         border-radius: 8px;\n\
         padding: 4px;\n\
         }}\n\
         popover listview {{\n\
         background-color: {combo_popup_bg};\n\
         color: {text_bright};\n\
         }}\n\
         popover listview > row:selected, popover listview > row:hover {{\n\
         background-color: {combo_popup_sel_bg};\n\
         color: {combo_popup_sel_text};\n\
         border-radius: 5px;\n\
         }}\n\
         checkbutton.salsa-eq-chk label {{\n\
         color: {accent_primary};\n\
         font-weight: 800;\n\
         font-size: {sec_sz}px;\n\
         }}\n\
         checkbutton > check {{\n\
         background-color: {bg_deep};\n\
         border: 2px solid {border_control};\n\
         border-radius: 4px;\n\
         min-width: 15px;\n\
         min-height: 15px;\n\
         }}\n\
         checkbutton > check:checked {{\n\
         background-color: {accent_primary};\n\
         border-color: {accent_primary};\n\
         color: #071014;\n\
         }}\n\
         textview, textview text {{\n\
         background-color: {bg_deep};\n\
         color: {text_primary};\n\
         font-family: monospace;\n\
         font-size: 11px;\n\
         }}\n\
         scrolledwindow {{\n\
         background-color: {bg_deep};\n\
         border: 1px solid {border_card};\n\
         border-radius: 6px;\n\
         }}\n\
         scrolledwindow.salsa-flat-scroll {{\n\
         background-color: transparent;\n\
         border: none;\n\
         }}\n\
         list, listview {{\n\
         background-color: {bg_deep};\n\
         color: {text_primary};\n\
         }}\n\
         row:selected {{\n\
         background-color: {bg_hover};\n\
         color: {text_bright};\n\
         }}\n\
         notebook > header {{\n\
         background-color: transparent;\n\
         border-bottom: 1px solid {border_card};\n\
         padding: 0px;\n\
         }}\n\
         notebook > header > tabs > tab {{\n\
         background-color: {bg_deep};\n\
         color: {text_secondary};\n\
         border: 1px solid {border_card};\n\
         border-bottom: none;\n\
         border-top-left-radius: 7px;\n\
         border-top-right-radius: 7px;\n\
         padding: 6px 14px;\n\
         margin-right: 3px;\n\
         font-weight: 700;\n\
         }}\n\
         notebook > header > tabs > tab:hover {{\n\
         color: {text_bright};\n\
         }}\n\
         notebook > header > tabs > tab:checked {{\n\
         background-color: {bg_control};\n\
         color: {accent_primary};\n\
         border-color: {border_control};\n\
         }}\n\
         notebook > stack {{\n\
         background-color: {bg_main};\n\
         border: 1px solid {border_card};\n\
         border-radius: 8px;\n\
         padding: 6px;\n\
         }}\n\
         separator {{\n\
         background-color: {border_card};\n\
         border: none;\n\
         min-height: 1px;\n\
         min-width: 1px;\n\
         }}\n",
        bg_main = t.bg_main,
        bg_card = t.bg_card,
        bg_deep = t.bg_deep,
        bg_canvas = t.bg_canvas,
        bg_control = t.bg_control,
        bg_hover = t.bg_hover,
        border_card = t.border_card,
        border_control = t.border_control,
        text_primary = t.text_primary,
        text_bright = t.text_bright,
        text_secondary = t.text_secondary,
        text_muted = t.text_muted,
        accent_primary = t.accent_primary,
        accent_secondary = t.accent_secondary,
        accent_warn = t.accent_warn,
        accent_mic = t.accent_mic,
        handle_border = t.handle_border,
        btn_primary_bg = t.btn_primary_bg,
        btn_primary_border = t.btn_primary_border,
        btn_primary_hover = t.btn_primary_hover,
        btn_primary_text = t.btn_primary_text,
        btn_accent_bg = t.btn_accent_bg,
        btn_accent_border = t.btn_accent_border,
        btn_accent_hover = t.btn_accent_hover,
        btn_accent_text = t.btn_accent_text,
        combo_popup_bg = t.combo_popup_bg,
        combo_popup_sel_bg = t.combo_popup_sel_bg,
        combo_popup_sel_text = t.combo_popup_sel_text,
        acc_tint = acc_tint,
        acc_border_tint = acc_border_tint,
        sec_tint = sec_tint,
        sec_border_tint = sec_border_tint,
        err_tint = err_tint,
    )
}

pub fn scale_gtk4_css_px(css: &str, scale: f64) -> String {
    if !scale.is_finite() || (scale - 1.0).abs() < 0.02 {
        return css.to_string();
    }
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + 64);
    let mut i = 0;
    while i < len {
        let prev_ok = i == 0
            || matches!(
                bytes[i - 1],
                b' ' | b'\t' | b'\n' | b'\r' | b':' | b'(' | b','
            );
        if prev_ok && bytes[i].is_ascii_digit() {
            let mut j = i;
            let mut saw_dot = false;
            while j < len {
                if bytes[j].is_ascii_digit() {
                    j += 1;
                } else if bytes[j] == b'.' && !saw_dot && j + 1 < len && bytes[j + 1].is_ascii_digit() {
                    saw_dot = true;
                    j += 1;
                } else {
                    break;
                }
            }
            if j + 2 <= len && &bytes[j..j + 2] == b"px" {
                let after_ok = j + 2 == len || !bytes[j + 2].is_ascii_alphanumeric();
                if after_ok {
                    if let Ok(val) = css[i..j].parse::<f64>() {
                        if saw_dot {
                            out.push_str(&format!("{:.1}px", val * scale));
                        } else if val == 0.0 {
                            out.push_str("0px");
                        } else if val == 1.0 {
                            let b = if scale >= 1.75 { 2 } else { 1 };
                            out.push_str(&format!("{b}px"));
                        } else {
                            let scaled = (val * scale).round().max(1.0) as i32;
                            out.push_str(&format!("{scaled}px"));
                        }
                        i = j + 2;
                        continue;
                    }
                }
            }
        }
        let ch = css[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn build_gtk4_css_scaled(t: &SalsaTheme, compact: bool, scale: f64) -> String {
    let base = build_gtk4_css(t, compact);
    scale_gtk4_css_px(&base, scale)
}

pub fn build_mini_preview_css_scaled(t: &SalsaTheme, scale: f64) -> String {
    let base = build_mini_preview_css(t);
    scale_gtk4_css_px(&base, scale)
}

pub fn build_mini_preview_css(t: &SalsaTheme) -> String {
    format!(
        ".salsa-mini-root {{\n\
         background-color: {bg_main};\n\
         color: {text_primary};\n\
         border: 2px solid {border_control};\n\
         border-radius: 10px;\n\
         padding: 8px;\n\
         font-size: 10px;\n\
         }}\n\
         .salsa-mini-root .salsa-card {{\n\
         background-color: {bg_card};\n\
         border: 1px solid {border_card};\n\
         border-radius: 7px;\n\
         padding: 6px 8px;\n\
         }}\n\
         .salsa-mini-root label {{\n\
         color: {text_primary};\n\
         }}\n\
         .salsa-mini-root .salsa-title-banner {{\n\
         color: {text_bright};\n\
         font-size: 13px;\n\
         font-weight: 800;\n\
         }}\n\
         .salsa-mini-root .salsa-sub-banner {{\n\
         color: {accent_secondary};\n\
         font-size: 9px;\n\
         font-weight: 600;\n\
         }}\n\
         .salsa-mini-root .salsa-studio-hdr {{\n\
         color: {accent_primary};\n\
         font-size: 10px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-mini-root .salsa-saved-chip {{\n\
         background-color: {bg_deep};\n\
         color: {accent_primary};\n\
         border: 1px solid {border_control};\n\
         border-radius: 5px;\n\
         padding: 2px 5px;\n\
         font-size: 9px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-mini-root .salsa-band-val {{\n\
         color: {text_bright};\n\
         font-size: 9px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-mini-root .salsa-band-dyn {{\n\
         color: {accent_warn};\n\
         font-family: monospace;\n\
         font-size: 8px;\n\
         }}\n\
         .salsa-mini-root .salsa-band-freq {{\n\
         color: {text_muted};\n\
         font-size: 9px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-mini-root .salsa-peaks-lbl {{\n\
         color: {accent_secondary};\n\
         font-family: monospace;\n\
         font-size: 9px;\n\
         font-weight: 700;\n\
         }}\n\
         .salsa-mini-root button, .salsa-mini-root dropdown > button {{\n\
         background-color: {bg_control};\n\
         color: {text_bright};\n\
         border: 1px solid {border_control};\n\
         border-radius: 5px;\n\
         padding: 2px 6px;\n\
         min-height: 16px;\n\
         font-size: 9px;\n\
         }}\n\
         .salsa-mini-root button.salsa-btn-primary {{\n\
         background-color: {btn_primary_bg};\n\
         color: {btn_primary_text};\n\
         border: 1px solid {btn_primary_border};\n\
         }}\n\
         .salsa-mini-root button.salsa-btn-accent {{\n\
         background-color: {btn_accent_bg};\n\
         color: {btn_accent_text};\n\
         border: 1px solid {btn_accent_border};\n\
         }}\n\
         .salsa-mini-root checkbutton > check {{\n\
         background-color: {bg_deep};\n\
         border: 1px solid {border_control};\n\
         min-width: 12px;\n\
         min-height: 12px;\n\
         }}\n\
         .salsa-mini-root checkbutton > check:checked {{\n\
         background-color: {accent_primary};\n\
         border-color: {accent_primary};\n\
         }}\n\
         .salsa-mini-root scale trough {{\n\
         background-color: {bg_control};\n\
         min-width: 4px;\n\
         min-height: 4px;\n\
         }}\n\
         .salsa-mini-root scale.horizontal slider {{\n\
         background-color: {accent_secondary};\n\
         border: 1px solid {handle_border};\n\
         min-width: 9px;\n\
         min-height: 9px;\n\
         }}\n\
         .salsa-mini-root scale.vertical slider {{\n\
         background-color: {accent_primary};\n\
         border: 1px solid {handle_border};\n\
         min-width: 9px;\n\
         min-height: 9px;\n\
         }}\n\
         .salsa-mini-root notebook > header > tabs > tab {{\n\
         background-color: {bg_card};\n\
         color: {text_muted};\n\
         border: 1px solid {border_card};\n\
         padding: 3px 8px;\n\
         font-size: 9px;\n\
         }}\n\
         .salsa-mini-root notebook > header > tabs > tab:checked {{\n\
         background-color: {bg_control};\n\
         color: {accent_primary};\n\
         border-color: {border_control};\n\
         }}\n\
         .salsa-mini-root notebook > stack, .salsa-mini-root .salsa-manual-box, .salsa-mini-root list {{\n\
         background-color: {bg_deep};\n\
         border: 1px solid {border_control};\n\
         }}\n",
        bg_main = t.bg_main,
        bg_card = t.bg_card,
        bg_deep = t.bg_deep,
        bg_control = t.bg_control,
        border_card = t.border_card,
        border_control = t.border_control,
        text_primary = t.text_primary,
        text_bright = t.text_bright,
        text_muted = t.text_muted,
        accent_primary = t.accent_primary,
        accent_secondary = t.accent_secondary,
        accent_warn = t.accent_warn,
        handle_border = t.handle_border,
        btn_primary_bg = t.btn_primary_bg,
        btn_primary_border = t.btn_primary_border,
        btn_primary_text = t.btn_primary_text,
        btn_accent_bg = t.btn_accent_bg,
        btn_accent_border = t.btn_accent_border,
        btn_accent_text = t.btn_accent_text,
    )
}

pub fn all_studio_theme_choices() -> Vec<String> {
    let mut out: Vec<String> = preset_theme_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    out.push(CUSTOM_THEME_NAME.to_string());
    out
}

pub fn all_tray_theme_choices() -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = vec!["Follow Studio Theme".to_string()];
    for name in preset_theme_names() {
        seen.insert(name);
        out.push(name.to_string());
    }
    for custom_name in [CUSTOM_THEME_NAME, "Custom Studio Theme"] {
        if !seen.contains(custom_name) {
            out.push(custom_name.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nine_presets_build_valid_css_and_roundtrip_json() {
        assert_eq!(THEME_COLOR_KEYS.len(), 41);
        let group_total: usize = THEME_TOKEN_GROUPS.iter().map(|(_, g)| g.len()).sum();
        assert_eq!(group_total, 41);

        let names = preset_theme_names();
        assert_eq!(names.len(), 9);
        for name in names {
            let theme = theme_by_name(name);
            let css = build_gtk4_css(&theme, false);
            assert!(css.contains(".salsa-card"));
            assert!(css.contains("scrollbar slider"));
            assert!(css.contains("scale.vertical slider"));
            assert!(!css.contains("max-height:"));
            assert!(css.contains(&theme.accent_primary));

            let mini_css = build_mini_preview_css(&theme);
            assert!(mini_css.contains(".salsa-mini-root"));

            let j = theme.to_json();
            let rt = SalsaTheme::from_json(&j, &theme_by_name("Salsa Midnight (Default)"));
            assert_eq!(rt.is_light, theme.is_light);
            for &k in THEME_COLOR_KEYS {
                assert_eq!(rt.get_token(k), theme.get_token(k), "mismatch on {k}");
            }

            let harm = derive_harmony_theme(&theme);
            for &k in THEME_COLOR_KEYS {
                assert_eq!(harm.get_token(k).len(), 7);
            }
        }
        assert_eq!(theme_by_name("Custom").name, "Custom");
        assert_eq!(theme_by_name("Custom Studio Theme").name, "Custom Studio Theme");
        let (r, g, b) = parse_hex_rgb("#00e676");
        assert!(r < 0.01 && g > 0.89 && b > 0.45);
        let _ = parse_hex_rgb("££££££");

        let default_t = theme_by_name("Salsa Midnight (Default)");
        let scaled = build_gtk4_css_scaled(&default_t, false, 1.5);
        assert!(scaled.contains(".salsa-tray-title"));
        assert!(scaled.contains("font-size: 23px;"));
        assert!(scaled.contains("border: 1px solid"));
        assert!(scaled.contains("border-radius: 0px;"));
        assert!(scaled.contains(&default_t.accent_primary));

        let mini_scaled = build_mini_preview_css_scaled(&default_t, 1.25);
        assert!(mini_scaled.contains("font-size: 13px;"));
        assert_eq!(normalize_font_scale_pct(40), UI_FONT_SCALE_MIN_PCT);
        assert_eq!(normalize_font_scale_pct(250), UI_FONT_SCALE_MAX_PCT);
        assert_eq!(apply_font_scale_step(115, 0), UI_FONT_SCALE_DEFAULT_PCT);
        assert_eq!(apply_font_scale_step(100, 5), 105);
        let j = JsonValue::Object(vec![("ui_font_scale_pct".into(), JsonValue::Int(125))]);
        assert_eq!(extract_font_scale_pct_from_json(&j), 125);
    }
}
