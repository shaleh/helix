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

/// Resolve the effective (fg, bg) color pair for a scope in a theme.
/// When the scope's style lacks a bg, falls back to the appropriate
/// context scope (e.g., `ui.background` for most scopes).
pub fn resolve_contrast_pair(theme: &Theme, scope: &str) -> (Option<Color>, Option<Color>) {
    let style = theme.get(scope);
    let fg = style.fg;
    let bg = style.bg.or_else(|| {
        let bg_scope = effective_bg_scope(scope);
        theme.try_get(bg_scope).and_then(|s| s.bg)
    });
    (fg, bg)
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
}
