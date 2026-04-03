use crate::graphics::Color;
use crate::theme::{ScopeCategory, Theme, DOCUMENTED_SCOPES};

/// WCAG 2.1 conformance level for a contrast ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WcagLevel {
    Fail,
    AA,
    AAA,
}

/// How a color was resolved to RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedColor {
    /// Known RGB value (true color or standard indexed palette).
    Exact(u8, u8, u8),
    /// Approximated from ANSI 16-color using standard xterm defaults.
    Estimated(u8, u8, u8),
    /// Cannot resolve (terminal default or unset).
    Unknown,
}

impl ResolvedColor {
    pub fn rgb(self) -> Option<(u8, u8, u8)> {
        match self {
            ResolvedColor::Exact(r, g, b) | ResolvedColor::Estimated(r, g, b) => Some((r, g, b)),
            ResolvedColor::Unknown => None,
        }
    }

    pub fn is_estimated(self) -> bool {
        matches!(self, ResolvedColor::Estimated(..))
    }
}

/// Result of computing contrast between two colors.
#[derive(Debug, Clone, Copy)]
pub enum ContrastResult {
    /// Ratio computed from known RGB values.
    Exact { ratio: f64, level: WcagLevel },
    /// Ratio computed using at least one estimated (ANSI approximation) color.
    Estimated { ratio: f64, level: WcagLevel },
    /// Cannot compute — one or both colors are terminal-dependent.
    Unknown,
}

impl ContrastResult {
    pub fn ratio(self) -> Option<f64> {
        match self {
            ContrastResult::Exact { ratio, .. } | ContrastResult::Estimated { ratio, .. } => {
                Some(ratio)
            }
            ContrastResult::Unknown => None,
        }
    }

    pub fn level(self) -> Option<WcagLevel> {
        match self {
            ContrastResult::Exact { level, .. } | ContrastResult::Estimated { level, .. } => {
                Some(level)
            }
            ContrastResult::Unknown => None,
        }
    }
}

// Standard xterm defaults for ANSI 16 colors.
// These match the default xterm color palette.
const ANSI_COLORS: [(u8, u8, u8); 16] = [
    (0, 0, 0),       // 0: Black
    (128, 0, 0),     // 1: Red (dark)
    (0, 128, 0),     // 2: Green (dark)
    (128, 128, 0),   // 3: Yellow (dark)
    (0, 0, 128),     // 4: Blue (dark)
    (128, 0, 128),   // 5: Magenta (dark)
    (0, 128, 128),   // 6: Cyan (dark)
    (192, 192, 192), // 7: White (light gray)
    (128, 128, 128), // 8: Bright black (gray)
    (255, 0, 0),     // 9: Bright red
    (0, 255, 0),     // 10: Bright green
    (255, 255, 0),   // 11: Bright yellow
    (0, 0, 255),     // 12: Bright blue
    (255, 0, 255),   // 13: Bright magenta
    (0, 255, 255),   // 14: Bright cyan
    (255, 255, 255), // 15: Bright white
];

/// Convert a single sRGB channel value (0–255) to linear light.
fn srgb_to_linear(value: u8) -> f64 {
    let v = value as f64 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative luminance per WCAG 2.1 (0.0 = black, 1.0 = white).
pub fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
}

/// WCAG 2.1 contrast ratio between two RGB colors (1.0–21.0).
pub fn contrast_ratio(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> f64 {
    let l1 = relative_luminance(fg.0, fg.1, fg.2);
    let l2 = relative_luminance(bg.0, bg.1, bg.2);
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Classify a contrast ratio into a WCAG conformance level.
pub fn wcag_level(ratio: f64) -> WcagLevel {
    if ratio >= 7.0 {
        WcagLevel::AAA
    } else if ratio >= 4.5 {
        WcagLevel::AA
    } else {
        WcagLevel::Fail
    }
}

/// Convert a 256-color indexed value to RGB.
/// Indices 0–15 are ANSI (terminal-dependent), 16–231 are the xterm
/// 6×6×6 color cube, 232–255 are a grayscale ramp.
fn indexed_to_rgb(index: u8) -> ResolvedColor {
    match index {
        0..=15 => {
            let (r, g, b) = ANSI_COLORS[index as usize];
            ResolvedColor::Estimated(r, g, b)
        }
        16..=231 => {
            let i = index - 16;
            let r_idx = i / 36;
            let g_idx = (i % 36) / 6;
            let b_idx = i % 6;
            let to_val = |idx: u8| if idx == 0 { 0 } else { 55 + 40 * idx };
            ResolvedColor::Exact(to_val(r_idx), to_val(g_idx), to_val(b_idx))
        }
        232..=255 => {
            let v = 8 + 10 * (index - 232);
            ResolvedColor::Exact(v, v, v)
        }
    }
}

/// Resolve a `Color` to an RGB triple.
pub fn resolve_color(color: Color) -> ResolvedColor {
    match color {
        Color::Rgb(r, g, b) => ResolvedColor::Exact(r, g, b),
        Color::Indexed(i) => indexed_to_rgb(i),
        Color::Black => ResolvedColor::Estimated(ANSI_COLORS[0].0, ANSI_COLORS[0].1, ANSI_COLORS[0].2),
        Color::Red => ResolvedColor::Estimated(ANSI_COLORS[1].0, ANSI_COLORS[1].1, ANSI_COLORS[1].2),
        Color::Green => ResolvedColor::Estimated(ANSI_COLORS[2].0, ANSI_COLORS[2].1, ANSI_COLORS[2].2),
        Color::Yellow => ResolvedColor::Estimated(ANSI_COLORS[3].0, ANSI_COLORS[3].1, ANSI_COLORS[3].2),
        Color::Blue => ResolvedColor::Estimated(ANSI_COLORS[4].0, ANSI_COLORS[4].1, ANSI_COLORS[4].2),
        Color::Magenta => ResolvedColor::Estimated(ANSI_COLORS[5].0, ANSI_COLORS[5].1, ANSI_COLORS[5].2),
        Color::Cyan => ResolvedColor::Estimated(ANSI_COLORS[6].0, ANSI_COLORS[6].1, ANSI_COLORS[6].2),
        Color::Gray => ResolvedColor::Estimated(ANSI_COLORS[8].0, ANSI_COLORS[8].1, ANSI_COLORS[8].2),
        Color::LightRed => ResolvedColor::Estimated(ANSI_COLORS[9].0, ANSI_COLORS[9].1, ANSI_COLORS[9].2),
        Color::LightGreen => ResolvedColor::Estimated(ANSI_COLORS[10].0, ANSI_COLORS[10].1, ANSI_COLORS[10].2),
        Color::LightYellow => ResolvedColor::Estimated(ANSI_COLORS[11].0, ANSI_COLORS[11].1, ANSI_COLORS[11].2),
        Color::LightBlue => ResolvedColor::Estimated(ANSI_COLORS[12].0, ANSI_COLORS[12].1, ANSI_COLORS[12].2),
        Color::LightMagenta => ResolvedColor::Estimated(ANSI_COLORS[13].0, ANSI_COLORS[13].1, ANSI_COLORS[13].2),
        Color::LightCyan => ResolvedColor::Estimated(ANSI_COLORS[14].0, ANSI_COLORS[14].1, ANSI_COLORS[14].2),
        Color::LightGray => ResolvedColor::Estimated(ANSI_COLORS[7].0, ANSI_COLORS[7].1, ANSI_COLORS[7].2),
        Color::White => ResolvedColor::Estimated(ANSI_COLORS[15].0, ANSI_COLORS[15].1, ANSI_COLORS[15].2),
        Color::Reset => ResolvedColor::Unknown,
    }
}

/// Compute the WCAG contrast between two `Color` values.
pub fn compute_contrast(fg: Color, bg: Color) -> ContrastResult {
    let fg_resolved = resolve_color(fg);
    let bg_resolved = resolve_color(bg);

    match (fg_resolved.rgb(), bg_resolved.rgb()) {
        (Some(fg_rgb), Some(bg_rgb)) => {
            let ratio = contrast_ratio(fg_rgb, bg_rgb);
            let level = wcag_level(ratio);
            if fg_resolved.is_estimated() || bg_resolved.is_estimated() {
                ContrastResult::Estimated { ratio, level }
            } else {
                ContrastResult::Exact { ratio, level }
            }
        }
        _ => ContrastResult::Unknown,
    }
}

/// Determine the background context scope for a given scope name.
fn effective_bg_scope(scope: &str) -> &str {
    if scope.starts_with("ui.statusline") {
        "ui.statusline"
    } else if scope.starts_with("ui.popup") || scope.starts_with("ui.menu") {
        "ui.popup"
    } else if scope.starts_with("ui.bufferline") {
        "ui.bufferline"
    } else {
        "ui.background"
    }
}

/// Why a scope's contrast could not be computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// The scope has no foreground color (uses terminal default or only modifiers).
    NoForeground,
    /// The scope has no background color and ui.background has none either.
    NoBackground,
    /// Both fg and bg are missing.
    NeitherColor,
    /// A color uses `Color::Reset` (terminal default).
    TerminalDefault,
}

impl UnknownReason {
    pub fn label(self) -> &'static str {
        match self {
            UnknownReason::NoForeground => "no foreground color set",
            UnknownReason::NoBackground => "no background color resolved",
            UnknownReason::NeitherColor => "no colors set (scope not defined)",
            UnknownReason::TerminalDefault => "uses terminal default color",
        }
    }

    pub fn recommendation(self) -> &'static str {
        match self {
            UnknownReason::NoForeground => "Add an explicit fg color to this scope or its parent.",
            UnknownReason::NoBackground => "Add a bg color to ui.background.",
            UnknownReason::NeitherColor => "Define this scope in the theme, or ensure a parent scope has a fg color.",
            UnknownReason::TerminalDefault => "Replace 'default' or terminal-dependent colors with explicit hex values (e.g., #rrggbb).",
        }
    }
}

/// Resolve the effective (fg, bg) color pair for a scope in a theme.
/// When the scope's style lacks a fg, falls back to `ui.text` fg.
/// When the scope's style lacks a bg, falls back to the appropriate
/// context scope (e.g., `ui.background` for most scopes).
pub fn resolve_contrast_pair(theme: &Theme, scope: &str) -> (Option<Color>, Option<Color>) {
    let style = theme.get(scope);
    let fg = style.fg.or_else(|| {
        // Fall back to ui.text fg — that's what most text actually renders with.
        theme.try_get("ui.text").and_then(|s| s.fg)
    });
    let bg = style.bg.or_else(|| {
        let bg_scope = effective_bg_scope(scope);
        theme.try_get(bg_scope).and_then(|s| s.bg)
    });
    (fg, bg)
}

/// Determine why a scope's contrast is unknown.
pub fn unknown_reason(theme: &Theme, scope: &str) -> UnknownReason {
    let style = theme.get(scope);
    let has_fg = style.fg.is_some()
        || theme.try_get("ui.text").and_then(|s| s.fg).is_some();
    let has_bg = style.bg.is_some()
        || theme
            .try_get(effective_bg_scope(scope))
            .and_then(|s| s.bg)
            .is_some();

    // Check if resolved colors are Color::Reset
    let (fg, bg) = resolve_contrast_pair(theme, scope);
    if let Some(fg_color) = fg {
        if matches!(resolve_color(fg_color), ResolvedColor::Unknown) {
            return UnknownReason::TerminalDefault;
        }
    }
    if let Some(bg_color) = bg {
        if matches!(resolve_color(bg_color), ResolvedColor::Unknown) {
            return UnknownReason::TerminalDefault;
        }
    }

    match (has_fg, has_bg) {
        (false, false) => UnknownReason::NeitherColor,
        (false, true) => UnknownReason::NoForeground,
        (true, false) => UnknownReason::NoBackground,
        (true, true) => UnknownReason::TerminalDefault, // Both resolved but to Reset
    }
}

/// Accessibility analysis for a single scope.
#[derive(Debug, Clone, Copy)]
pub struct ScopeContrast {
    pub name: &'static str,
    pub category: ScopeCategory,
    pub result: ContrastResult,
    pub level_label: &'static str,
}

/// Summary of a theme's WCAG accessibility.
#[derive(Debug, Clone)]
pub struct AccessibilityReport {
    pub theme_name: String,
    pub scopes: Vec<ScopeContrast>,
    pub pass_aaa: usize,
    pub pass_aa: usize,
    pub fail: usize,
    pub unknown: usize,
}

impl AccessibilityReport {
    /// Analyze a theme against WCAG contrast requirements.
    pub fn analyze(theme: &Theme) -> Self {
        let mut pass_aaa = 0;
        let mut pass_aa = 0;
        let mut fail = 0;
        let mut unknown = 0;

        let mut scopes: Vec<ScopeContrast> = DOCUMENTED_SCOPES
            .iter()
            .map(|scope| {
                let (fg, bg) = resolve_contrast_pair(theme, scope.name);
                let result = match (fg, bg) {
                    (Some(fg_color), Some(bg_color)) => compute_contrast(fg_color, bg_color),
                    _ => ContrastResult::Unknown,
                };

                match result.level() {
                    Some(WcagLevel::AAA) => pass_aaa += 1,
                    Some(WcagLevel::AA) => pass_aa += 1,
                    Some(WcagLevel::Fail) => fail += 1,
                    None => unknown += 1,
                }

                let level_label = match result {
                    ContrastResult::Exact { level, .. } => match level {
                        WcagLevel::AAA => "AAA",
                        WcagLevel::AA => "AA",
                        WcagLevel::Fail => "FAIL",
                    },
                    ContrastResult::Estimated { level, .. } => match level {
                        WcagLevel::AAA => "~AAA",
                        WcagLevel::AA => "~AA",
                        WcagLevel::Fail => "~FAIL",
                    },
                    ContrastResult::Unknown => "???",
                };

                ScopeContrast {
                    name: scope.name,
                    category: scope.category,
                    result,
                    level_label,
                }
            })
            .collect();

        // Sort by ratio ascending (worst contrast first), unknowns last.
        scopes.sort_by(|a, b| {
            let ra = a.result.ratio().unwrap_or(f64::MAX);
            let rb = b.result.ratio().unwrap_or(f64::MAX);
            ra.partial_cmp(&rb).unwrap()
        });

        AccessibilityReport {
            theme_name: theme.name().to_string(),
            scopes,
            pass_aaa,
            pass_aa,
            fail,
            unknown,
        }
    }

    /// Total computable scopes (excludes unknown).
    pub fn computable(&self) -> usize {
        self.pass_aaa + self.pass_aa + self.fail
    }

    /// Percentage of computable scopes passing AA (includes AAA).
    pub fn aa_percent(&self) -> f64 {
        let computable = self.computable();
        if computable == 0 {
            return 0.0;
        }
        (self.pass_aaa + self.pass_aa) as f64 / computable as f64 * 100.0
    }

    /// Percentage of computable scopes passing AAA.
    pub fn aaa_percent(&self) -> f64 {
        let computable = self.computable();
        if computable == 0 {
            return 0.0;
        }
        self.pass_aaa as f64 / computable as f64 * 100.0
    }
}

// ── Readability tiers ──────────────────────────────────────────────

/// Human-readable description of a contrast ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readability {
    EasyToRead,
    Readable,
    HardToRead,
    VeryHardToRead,
    NearlyInvisible,
}

impl Readability {
    pub fn from_ratio(ratio: f64) -> Self {
        if ratio >= 7.0 {
            Readability::EasyToRead
        } else if ratio >= 4.5 {
            Readability::Readable
        } else if ratio >= 3.0 {
            Readability::HardToRead
        } else if ratio >= 1.5 {
            Readability::VeryHardToRead
        } else {
            Readability::NearlyInvisible
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Readability::EasyToRead => "easy to read",
            Readability::Readable => "readable",
            Readability::HardToRead => "hard to read",
            Readability::VeryHardToRead => "very hard to read",
            Readability::NearlyInvisible => "nearly invisible",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Readability::EasyToRead => "Comfortable for everyone including users with low vision.",
            Readability::Readable => "Meets the minimum accessibility standard. Most users will be fine.",
            Readability::HardToRead => "Many users will struggle, especially on lower-quality displays or in bright rooms.",
            Readability::VeryHardToRead => "Most users will have difficulty reading this text.",
            Readability::NearlyInvisible => "These colors are almost the same. Text will be extremely hard to see.",
        }
    }
}

// ── OKLAB color space ──────────────────────────────────────────────

/// A color in the OKLAB perceptual color space.
#[derive(Debug, Clone, Copy)]
pub struct Oklab {
    /// Lightness (0.0 = black, 1.0 = white).
    pub l: f64,
    /// Green-red axis.
    pub a: f64,
    /// Blue-yellow axis.
    pub b: f64,
}

/// Convert linear RGB (0.0–1.0 per channel) to OKLAB.
fn linear_rgb_to_oklab(r: f64, g: f64, b: f64) -> Oklab {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    Oklab {
        l: 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        a: 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    }
}

/// Convert OKLAB to linear RGB (0.0–1.0 per channel).
fn oklab_to_linear_rgb(lab: Oklab) -> (f64, f64, f64) {
    let l_ = lab.l + 0.3963377774 * lab.a + 0.2158037573 * lab.b;
    let m_ = lab.l - 0.1055613458 * lab.a - 0.0638541728 * lab.b;
    let s_ = lab.l - 0.0894841775 * lab.a - 1.2914855480 * lab.b;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;

    (r, g, b)
}

/// Convert linear light value to sRGB (0–255).
fn linear_to_srgb(v: f64) -> u8 {
    let clamped = v.clamp(0.0, 1.0);
    let srgb = if clamped <= 0.0031308 {
        12.92 * clamped
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5) as u8
}

/// Convert sRGB (0–255) to OKLAB.
pub fn srgb_to_oklab(r: u8, g: u8, b: u8) -> Oklab {
    linear_rgb_to_oklab(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b))
}

/// Convert OKLAB to sRGB (0–255), clamping out-of-gamut values.
pub fn oklab_to_srgb(lab: Oklab) -> (u8, u8, u8) {
    let (r, g, b) = oklab_to_linear_rgb(lab);
    (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

// ── Color suggestions ──────────────────────────────────────────────

/// Which color was adjusted in a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjustTarget {
    Foreground,
    Background,
}

/// A suggested color adjustment to improve contrast.
#[derive(Debug, Clone, Copy)]
pub struct ColorSuggestion {
    pub target: AdjustTarget,
    pub original: (u8, u8, u8),
    pub suggested: (u8, u8, u8),
    pub original_ratio: f64,
    pub suggested_ratio: f64,
    pub target_level: WcagLevel,
}

/// Find the minimum OKLAB lightness adjustment to reach a target contrast
/// ratio against a fixed background color. Preserves hue and chroma.
///
/// Returns `None` if the target cannot be reached (e.g., both colors
/// are at the extreme of the lightness range).
pub fn suggest_lightness_fix(
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    target_ratio: f64,
) -> Option<(u8, u8, u8)> {
    let bg_lum = relative_luminance(bg.0, bg.1, bg.2);
    let fg_lab = srgb_to_oklab(fg.0, fg.1, fg.2);

    // Determine direction: lighten fg if bg is dark, darken if bg is light.
    let bg_is_dark = bg_lum < 0.5;

    // Binary search for the minimum lightness change.
    let (mut lo, mut hi) = if bg_is_dark {
        (fg_lab.l, 1.0) // search lighter
    } else {
        (0.0, fg_lab.l) // search darker
    };

    // Check if the target is reachable at the extreme.
    let extreme = Oklab { l: if bg_is_dark { hi } else { lo }, ..fg_lab };
    let extreme_rgb = oklab_to_srgb(extreme);
    let extreme_ratio = contrast_ratio(extreme_rgb, bg);
    if extreme_ratio < target_ratio {
        return None; // Target unreachable even at max/min lightness.
    }

    for _ in 0..32 {
        let mid = (lo + hi) / 2.0;
        let candidate = Oklab { l: mid, ..fg_lab };
        let rgb = oklab_to_srgb(candidate);
        let ratio = contrast_ratio(rgb, bg);

        if bg_is_dark && ratio < target_ratio {
            lo = mid; // need lighter
        } else if bg_is_dark {
            hi = mid; // can be darker
        } else if ratio < target_ratio {
            hi = mid; // need darker
        } else {
            lo = mid; // can be lighter
        }
    }

    let result_l = if bg_is_dark { hi } else { lo };
    let result = Oklab { l: result_l, ..fg_lab };
    let rgb = oklab_to_srgb(result);

    // Verify the suggestion actually meets the target.
    if contrast_ratio(rgb, bg) >= target_ratio {
        Some(rgb)
    } else {
        None
    }
}

/// Find the minimum OKLAB lightness adjustment to a background color
/// to reach a target contrast ratio against a fixed foreground.
/// Tries both darkening and lightening the bg, returns whichever
/// requires the smallest change from the original.
pub fn suggest_bg_lightness_fix(
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    target_ratio: f64,
) -> Option<(u8, u8, u8)> {
    let bg_lab = srgb_to_oklab(bg.0, bg.1, bg.2);

    let try_direction = |search_lo: f64, search_hi: f64, darken: bool| -> Option<(u8, u8, u8)> {
        let extreme_l = if darken { search_lo } else { search_hi };
        let extreme = Oklab { l: extreme_l, ..bg_lab };
        let extreme_rgb = oklab_to_srgb(extreme);
        if contrast_ratio(fg, extreme_rgb) < target_ratio {
            return None;
        }

        let mut lo = search_lo;
        let mut hi = search_hi;
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            let candidate = Oklab { l: mid, ..bg_lab };
            let rgb = oklab_to_srgb(candidate);
            let ratio = contrast_ratio(fg, rgb);

            if darken {
                if ratio < target_ratio {
                    hi = mid;
                } else {
                    lo = mid;
                }
            } else if ratio < target_ratio {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        let result_l = if darken { lo } else { hi };
        let result = Oklab { l: result_l, ..bg_lab };
        let rgb = oklab_to_srgb(result);

        if contrast_ratio(fg, rgb) >= target_ratio {
            Some(rgb)
        } else {
            None
        }
    };

    // Try both directions
    let darker = try_direction(0.0, bg_lab.l, true);
    let lighter = try_direction(bg_lab.l, 1.0, false);

    // Pick whichever is closest to the original bg
    match (darker, lighter) {
        (Some(d), Some(l)) => {
            let d_dist = (srgb_to_oklab(d.0, d.1, d.2).l - bg_lab.l).abs();
            let l_dist = (srgb_to_oklab(l.0, l.1, l.2).l - bg_lab.l).abs();
            Some(if d_dist <= l_dist { d } else { l })
        }
        (Some(d), None) => Some(d),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

/// Generate suggestions for a failing fg/bg pair.
/// Returns both foreground and background adjustment options at AA level.
pub fn suggest_fixes(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> Vec<ColorSuggestion> {
    let original_ratio = contrast_ratio(fg, bg);
    let mut suggestions = Vec::new();

    // Foreground adjustments
    if original_ratio < 4.5 {
        if let Some(suggested) = suggest_lightness_fix(fg, bg, 4.5) {
            suggestions.push(ColorSuggestion {
                target: AdjustTarget::Foreground,
                original: fg,
                suggested,
                original_ratio,
                suggested_ratio: contrast_ratio(suggested, bg),
                target_level: WcagLevel::AA,
            });
        }
    }
    if original_ratio < 7.0 {
        if let Some(suggested) = suggest_lightness_fix(fg, bg, 7.0) {
            suggestions.push(ColorSuggestion {
                target: AdjustTarget::Foreground,
                original: fg,
                suggested,
                original_ratio,
                suggested_ratio: contrast_ratio(suggested, bg),
                target_level: WcagLevel::AAA,
            });
        }
    }

    // Background adjustments
    if original_ratio < 4.5 {
        if let Some(suggested) = suggest_bg_lightness_fix(fg, bg, 4.5) {
            suggestions.push(ColorSuggestion {
                target: AdjustTarget::Background,
                original: bg,
                suggested,
                original_ratio,
                suggested_ratio: contrast_ratio(fg, suggested),
                target_level: WcagLevel::AA,
            });
        }
    }
    if original_ratio < 7.0 {
        if let Some(suggested) = suggest_bg_lightness_fix(fg, bg, 7.0) {
            suggestions.push(ColorSuggestion {
                target: AdjustTarget::Background,
                original: bg,
                suggested,
                original_ratio,
                suggested_ratio: contrast_ratio(fg, suggested),
                target_level: WcagLevel::AAA,
            });
        }
    }

    suggestions
}

/// Format an RGB color as a hex string.
pub fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

/// ANSI true-color escape sequence for a colored block.
pub fn ansi_fg_block(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m\u{2588}\u{2588}\x1b[0m")
}

/// ANSI true-color escape for text with fg on bg.
pub fn ansi_colored_text(text: &str, fg: (u8, u8, u8), bg: (u8, u8, u8)) -> String {
    format!(
        "\x1b[38;2;{};{};{};48;2;{};{};{}m{}\x1b[0m",
        fg.0, fg.1, fg.2, bg.0, bg.1, bg.2, text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_on_black_is_21_to_1() {
        let ratio = contrast_ratio((255, 255, 255), (0, 0, 0));
        assert!((ratio - 21.0).abs() < 0.01, "Expected ~21.0, got {ratio}");
    }

    #[test]
    fn black_on_white_is_21_to_1() {
        let ratio = contrast_ratio((0, 0, 0), (255, 255, 255));
        assert!((ratio - 21.0).abs() < 0.01, "Expected ~21.0, got {ratio}");
    }

    #[test]
    fn same_color_is_1_to_1() {
        let ratio = contrast_ratio((128, 128, 128), (128, 128, 128));
        assert!((ratio - 1.0).abs() < 0.01, "Expected 1.0, got {ratio}");
    }

    #[test]
    fn wcag_level_classification() {
        assert_eq!(wcag_level(21.0), WcagLevel::AAA);
        assert_eq!(wcag_level(7.0), WcagLevel::AAA);
        assert_eq!(wcag_level(6.9), WcagLevel::AA);
        assert_eq!(wcag_level(4.5), WcagLevel::AA);
        assert_eq!(wcag_level(4.4), WcagLevel::Fail);
        assert_eq!(wcag_level(1.0), WcagLevel::Fail);
    }

    #[test]
    fn resolve_rgb_is_exact() {
        assert_eq!(
            resolve_color(Color::Rgb(100, 200, 50)),
            ResolvedColor::Exact(100, 200, 50)
        );
    }

    #[test]
    fn resolve_ansi_is_estimated() {
        assert!(resolve_color(Color::Red).is_estimated());
        assert!(resolve_color(Color::White).is_estimated());
    }

    #[test]
    fn resolve_reset_is_unknown() {
        assert_eq!(resolve_color(Color::Reset), ResolvedColor::Unknown);
    }

    #[test]
    fn indexed_cube_colors() {
        // Index 16 = rgb(0, 0, 0) in the cube
        assert_eq!(indexed_to_rgb(16), ResolvedColor::Exact(0, 0, 0));
        // Index 231 = rgb(255, 255, 255) in the cube
        assert_eq!(indexed_to_rgb(231), ResolvedColor::Exact(255, 255, 255));
        // Index 196 = rgb(255, 0, 0) — pure red in the cube
        // 196 - 16 = 180; 180/36=5, 180%36=0, 0/6=0, 0%6=0
        // r=55+40*5=255, g=0, b=0
        assert_eq!(indexed_to_rgb(196), ResolvedColor::Exact(255, 0, 0));
    }

    #[test]
    fn indexed_grayscale() {
        // Index 232 = 8 (darkest gray)
        assert_eq!(indexed_to_rgb(232), ResolvedColor::Exact(8, 8, 8));
        // Index 255 = 8 + 10*23 = 238 (lightest gray)
        assert_eq!(indexed_to_rgb(255), ResolvedColor::Exact(238, 238, 238));
    }

    #[test]
    fn contrast_with_unknown_is_unknown() {
        let result = compute_contrast(Color::Reset, Color::Rgb(255, 255, 255));
        assert!(matches!(result, ContrastResult::Unknown));
    }

    #[test]
    fn contrast_rgb_pair_is_exact() {
        let result = compute_contrast(Color::Rgb(255, 255, 255), Color::Rgb(0, 0, 0));
        match result {
            ContrastResult::Exact { ratio, level } => {
                assert!((ratio - 21.0).abs() < 0.01);
                assert_eq!(level, WcagLevel::AAA);
            }
            _ => panic!("Expected Exact, got {result:?}"),
        }
    }

    #[test]
    fn contrast_with_ansi_is_estimated() {
        let result = compute_contrast(Color::White, Color::Black);
        assert!(matches!(result, ContrastResult::Estimated { .. }));
    }

    #[test]
    fn default_theme_accessibility_report() {
        use crate::theme::DEFAULT_THEME;
        let report = AccessibilityReport::analyze(&DEFAULT_THEME);
        assert!(!report.scopes.is_empty());
        assert!(
            report.computable() > 0,
            "Default theme should have computable contrast pairs"
        );
    }

    #[test]
    fn oklab_round_trip_white() {
        let lab = srgb_to_oklab(255, 255, 255);
        let (r, g, b) = oklab_to_srgb(lab);
        assert_eq!((r, g, b), (255, 255, 255));
    }

    #[test]
    fn oklab_round_trip_black() {
        let lab = srgb_to_oklab(0, 0, 0);
        let (r, g, b) = oklab_to_srgb(lab);
        assert_eq!((r, g, b), (0, 0, 0));
    }

    #[test]
    fn oklab_round_trip_color() {
        let lab = srgb_to_oklab(100, 200, 50);
        let (r, g, b) = oklab_to_srgb(lab);
        // Allow ±1 for rounding
        assert!((r as i16 - 100).abs() <= 1);
        assert!((g as i16 - 200).abs() <= 1);
        assert!((b as i16 - 50).abs() <= 1);
    }

    #[test]
    fn suggest_fix_for_low_contrast() {
        // Light gray on dark background — should suggest lighter fg
        let fg = (100, 100, 100);
        let bg = (30, 30, 30);
        let ratio = contrast_ratio(fg, bg);
        assert!(ratio < 4.5, "Test setup: should be below AA");

        let suggestions = suggest_fixes(fg, bg);
        assert!(!suggestions.is_empty(), "Should produce at least one suggestion");

        let aa = &suggestions[0];
        assert!(aa.suggested_ratio >= 4.5, "AA suggestion should meet 4.5:1");
        assert_eq!(aa.target_level, WcagLevel::AA);
    }

    #[test]
    fn suggest_fix_preserves_hue() {
        // A distinctly colored fg
        let fg = (180, 50, 50); // reddish
        let bg = (20, 20, 20);

        if let Some(suggested) = suggest_lightness_fix(fg, bg, 4.5) {
            // The suggested color should still be reddish (r > g, r > b)
            assert!(
                suggested.0 > suggested.1 && suggested.0 > suggested.2,
                "Suggested {:?} should preserve reddish hue",
                suggested
            );
        }
    }

    #[test]
    fn suggest_bg_fix_for_low_contrast() {
        // Light fg on medium-dark bg
        let fg = (136, 192, 208); // #88C0D0
        let bg = (67, 76, 94);    // #434C5E
        let ratio = contrast_ratio(fg, bg);
        assert!(ratio < 4.5, "Test setup: ratio {ratio} should be below AA");

        let result = suggest_bg_lightness_fix(fg, bg, 4.5);
        assert!(
            result.is_some(),
            "Should be able to darken bg to reach AA, ratio was {ratio}"
        );
        if let Some(suggested) = result {
            let new_ratio = contrast_ratio(fg, suggested);
            assert!(
                new_ratio >= 4.5,
                "Suggested bg {:?} gives ratio {new_ratio}, should be >= 4.5",
                suggested
            );
        }
    }

    #[test]
    fn suggest_fixes_includes_both_fg_and_bg() {
        let fg = (136, 192, 208); // light cyan
        let bg = (67, 76, 94);    // medium dark
        let suggestions = suggest_fixes(fg, bg);
        let has_fg = suggestions.iter().any(|s| s.target == AdjustTarget::Foreground);
        let has_bg = suggestions.iter().any(|s| s.target == AdjustTarget::Background);
        assert!(has_fg, "Should have fg suggestions");
        assert!(has_bg, "Should have bg suggestions, got: {:?}", suggestions);
    }

    #[test]
    fn readability_tiers() {
        assert_eq!(Readability::from_ratio(21.0), Readability::EasyToRead);
        assert_eq!(Readability::from_ratio(7.0), Readability::EasyToRead);
        assert_eq!(Readability::from_ratio(5.0), Readability::Readable);
        assert_eq!(Readability::from_ratio(4.5), Readability::Readable);
        assert_eq!(Readability::from_ratio(3.5), Readability::HardToRead);
        assert_eq!(Readability::from_ratio(2.0), Readability::VeryHardToRead);
        assert_eq!(Readability::from_ratio(1.2), Readability::NearlyInvisible);
    }
}
