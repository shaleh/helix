use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str,
};

use anyhow::{anyhow, Result};
use helix_core::{hashmap, syntax::Highlight};
use helix_loader::merge_toml_values;
use log::warn;
use once_cell::sync::Lazy;
use serde::{Deserialize, Deserializer};
use toml::{map::Map, Value};

use crate::graphics::UnderlineStyle;
pub use crate::graphics::{Color, Modifier, Style};

pub static DEFAULT_THEME_DATA: Lazy<Value> = Lazy::new(|| {
    let bytes = include_bytes!("../../theme.toml");
    toml::from_str(str::from_utf8(bytes).unwrap()).expect("Failed to parse base default theme")
});

pub static BASE16_DEFAULT_THEME_DATA: Lazy<Value> = Lazy::new(|| {
    let bytes = include_bytes!("../../base16_theme.toml");
    toml::from_str(str::from_utf8(bytes).unwrap()).expect("Failed to parse base 16 default theme")
});

pub static DEFAULT_THEME: Lazy<Theme> = Lazy::new(|| Theme {
    name: "default".into(),
    ..Theme::from(DEFAULT_THEME_DATA.clone())
});

pub static BASE16_DEFAULT_THEME: Lazy<Theme> = Lazy::new(|| Theme {
    name: "base16_default".into(),
    ..Theme::from(BASE16_DEFAULT_THEME_DATA.clone())
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    Dark,
    Light,
}

#[cfg(feature = "term")]
impl From<termina::escape::csi::ThemeMode> for Mode {
    fn from(mode: termina::escape::csi::ThemeMode) -> Self {
        match mode {
            termina::escape::csi::ThemeMode::Dark => Self::Dark,
            termina::escape::csi::ThemeMode::Light => Self::Light,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    light: String,
    dark: String,
    /// A theme to choose when the terminal did not declare either light or dark mode.
    /// When not specified the dark theme is preferred.
    fallback: Option<String>,
}

impl Config {
    pub fn choose(&self, preference: Option<Mode>) -> &str {
        match preference {
            Some(Mode::Light) => &self.light,
            Some(Mode::Dark) => &self.dark,
            None => self.fallback.as_ref().unwrap_or(&self.dark),
        }
    }
}

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged, deny_unknown_fields, rename_all = "kebab-case")]
        enum InnerConfig {
            Constant(String),
            Adaptive {
                dark: String,
                light: String,
                fallback: Option<String>,
            },
        }

        let inner = InnerConfig::deserialize(deserializer)?;

        let (light, dark, fallback) = match inner {
            InnerConfig::Constant(theme) => (theme.clone(), theme.clone(), None),
            InnerConfig::Adaptive {
                light,
                dark,
                fallback,
            } => (light, dark, fallback),
        };

        Ok(Self {
            light,
            dark,
            fallback,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Loader {
    /// Theme directories to search from highest to lowest priority
    theme_dirs: Vec<PathBuf>,
}
impl Loader {
    /// Creates a new loader that can load themes from multiple directories.
    ///
    /// The provided directories should be ordered from highest to lowest priority.
    /// The directories will have their "themes" subdirectory searched.
    pub fn new(dirs: &[PathBuf]) -> Self {
        Self {
            theme_dirs: dirs.iter().map(|p| p.join("themes")).collect(),
        }
    }

    /// Loads a theme searching directories in priority order.
    pub fn load(&self, name: &str) -> Result<Theme> {
        let (theme, warnings) = self.load_with_warnings(name)?;

        for warning in warnings {
            warn!("Theme '{}': {}", name, warning);
        }

        Ok(theme)
    }

    /// Loads a theme searching directories in priority order, returning any warnings
    pub fn load_with_warnings(&self, name: &str) -> Result<(Theme, Vec<String>)> {
        if name == "default" {
            return Ok((self.default(), Vec::new()));
        }
        if name == "base16_default" {
            return Ok((self.base16_default(), Vec::new()));
        }

        let mut visited_paths = HashSet::new();
        let (theme, warnings) = self
            .load_theme(name, &mut visited_paths)
            .map(Theme::from_toml)?;

        let theme = Theme {
            name: name.into(),
            ..theme
        };
        Ok((theme, warnings))
    }

    /// Recursively load a theme, merging with any inherited parent themes.
    ///
    /// The paths that have been visited in the inheritance hierarchy are tracked
    /// to detect and avoid cycling.
    ///
    /// It is possible for one file to inherit from another file with the same name
    /// so long as the second file is in a themes directory with lower priority.
    /// However, it is not recommended that users do this as it will make tracing
    /// errors more difficult.
    fn load_theme(&self, name: &str, visited_paths: &mut HashSet<PathBuf>) -> Result<Value> {
        let path = self.path(name, visited_paths)?;

        let theme_toml = self.load_toml(path)?;

        let inherits = theme_toml.get("inherits");

        let theme_toml = if let Some(parent_theme_name) = inherits {
            let parent_theme_name = parent_theme_name.as_str().ok_or_else(|| {
                anyhow!("Expected 'inherits' to be a string: {}", parent_theme_name)
            })?;

            let parent_theme_toml = match parent_theme_name {
                // load default themes's toml from const.
                "default" => DEFAULT_THEME_DATA.clone(),
                "base16_default" => BASE16_DEFAULT_THEME_DATA.clone(),
                _ => self.load_theme(parent_theme_name, visited_paths)?,
            };

            self.merge_themes(parent_theme_toml, theme_toml)
        } else {
            theme_toml
        };

        Ok(theme_toml)
    }

    pub fn read_names(path: &Path) -> Vec<String> {
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        let entry = entry.ok()?;
                        let path = entry.path();
                        (path.extension()? == "toml")
                            .then(|| path.file_stem().unwrap().to_string_lossy().into_owned())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // merge one theme into the parent theme
    fn merge_themes(&self, parent_theme_toml: Value, theme_toml: Value) -> Value {
        let parent_palette = parent_theme_toml.get("palette");
        let palette = theme_toml.get("palette");

        // handle the table separately since it needs a `merge_depth` of 2
        // this would conflict with the rest of the theme merge strategy
        let palette_values = match (parent_palette, palette) {
            (Some(parent_palette), Some(palette)) => {
                merge_toml_values(parent_palette.clone(), palette.clone(), 2)
            }
            (Some(parent_palette), None) => parent_palette.clone(),
            (None, Some(palette)) => palette.clone(),
            (None, None) => Map::new().into(),
        };

        // add the palette correctly as nested table
        let mut palette = Map::new();
        palette.insert(String::from("palette"), palette_values);

        // merge the theme into the parent theme
        let theme = merge_toml_values(parent_theme_toml, theme_toml, 1);
        // merge the before specially handled palette into the theme
        merge_toml_values(theme, palette.into(), 1)
    }

    // Loads the theme data as `toml::Value`
    fn load_toml(&self, path: PathBuf) -> Result<Value> {
        let data = std::fs::read_to_string(path)?;
        let value = toml::from_str(&data)?;

        Ok(value)
    }

    /// Returns the path to the theme with the given name
    ///
    /// Ignores paths already visited and follows directory priority order.
    fn path(&self, name: &str, visited_paths: &mut HashSet<PathBuf>) -> Result<PathBuf> {
        let filename = format!("{}.toml", name);

        let mut cycle_found = false; // track if there was a path, but it was in a cycle
        self.theme_dirs
            .iter()
            .find_map(|dir| {
                let path = dir.join(&filename);
                if !path.exists() {
                    None
                } else if visited_paths.contains(&path) {
                    // Avoiding cycle, continuing to look in lower priority directories
                    cycle_found = true;
                    None
                } else {
                    visited_paths.insert(path.clone());
                    Some(path)
                }
            })
            .ok_or_else(|| {
                if cycle_found {
                    anyhow!("Cycle found in inheriting: {}", name)
                } else {
                    anyhow!("File not found for: {}", name)
                }
            })
    }

    pub fn default_theme(&self, true_color: bool) -> Theme {
        if true_color {
            self.default()
        } else {
            self.base16_default()
        }
    }

    /// Returns the default theme
    pub fn default(&self) -> Theme {
        DEFAULT_THEME.clone()
    }

    /// Returns the alternative 16-color default theme
    pub fn base16_default(&self) -> Theme {
        BASE16_DEFAULT_THEME.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Theme {
    name: String,

    // UI styles are stored in a HashMap
    styles: HashMap<String, Style>,
    // tree-sitter highlight styles are stored in a Vec to optimize lookups
    scopes: Vec<String>,
    highlights: Vec<Style>,
    rainbow_length: usize,
}

impl From<Value> for Theme {
    fn from(value: Value) -> Self {
        let (theme, warnings) = Theme::from_toml(value);
        for warning in warnings {
            warn!("{}", warning);
        }
        theme
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Map::<String, Value>::deserialize(deserializer)?;
        let (theme, warnings) = Theme::from_keys(values);
        for warning in warnings {
            warn!("{}", warning);
        }
        Ok(theme)
    }
}

#[allow(clippy::type_complexity)]
fn build_theme_values(
    mut values: Map<String, Value>,
) -> (
    HashMap<String, Style>,
    Vec<String>,
    Vec<Style>,
    usize,
    Vec<String>,
) {
    let mut styles = HashMap::new();
    let mut scopes = Vec::new();
    let mut highlights = Vec::new();
    let mut rainbow_length = 0;

    let mut warnings = Vec::new();

    // TODO: alert user of parsing failures in editor
    let palette = values
        .remove("palette")
        .map(|value| {
            ThemePalette::try_from(value).unwrap_or_else(|err| {
                warnings.push(err);
                ThemePalette::default()
            })
        })
        .unwrap_or_default();
    // remove inherits from value to prevent errors
    let _ = values.remove("inherits");
    styles.reserve(values.len());
    scopes.reserve(values.len());
    highlights.reserve(values.len());

    for (i, style) in values
        .remove("rainbow")
        .and_then(|value| match palette.parse_style_array(value) {
            Ok(styles) => Some(styles),
            Err(err) => {
                warnings.push(err);
                None
            }
        })
        .unwrap_or_else(default_rainbow)
        .into_iter()
        .enumerate()
    {
        let name = format!("rainbow.{i}");
        styles.insert(name.clone(), style);
        scopes.push(name);
        highlights.push(style);
        rainbow_length += 1;
    }

    for (name, style_value) in values {
        let mut style = Style::default();
        if let Err(err) = palette.parse_style(&mut style, style_value) {
            warnings.push(format!("Failed to parse style for key {name:?}. {err}"));
        }

        // these are used both as UI and as highlights
        styles.insert(name.clone(), style);
        scopes.push(name);
        highlights.push(style);
    }

    (styles, scopes, highlights, rainbow_length, warnings)
}

fn default_rainbow() -> Vec<Style> {
    vec![
        Style::default().fg(Color::Red),
        Style::default().fg(Color::Yellow),
        Style::default().fg(Color::Green),
        Style::default().fg(Color::Blue),
        Style::default().fg(Color::Cyan),
        Style::default().fg(Color::Magenta),
    ]
}
impl Theme {
    /// To allow `Highlight` to represent arbitrary RGB colors without turning it into an enum,
    /// we interpret the last 256^3 numbers as RGB.
    const RGB_START: u32 = (u32::MAX << (8 + 8 + 8)) - 1 - (u32::MAX - Highlight::MAX);

    /// Interpret a Highlight with the RGB foreground
    fn decode_rgb_highlight(highlight: Highlight) -> Option<(u8, u8, u8)> {
        (highlight.get() > Self::RGB_START).then(|| {
            let [b, g, r, ..] = (highlight.get() + 1).to_le_bytes();
            (r, g, b)
        })
    }

    /// Create a Highlight that represents an RGB color
    pub fn rgb_highlight(r: u8, g: u8, b: u8) -> Highlight {
        // -1 because highlight is "non-max": u32::MAX is reserved for the null pointer
        // optimization.
        Highlight::new(u32::from_le_bytes([b, g, r, u8::MAX]) - 1)
    }

    #[inline]
    pub fn highlight(&self, highlight: Highlight) -> Style {
        if let Some((red, green, blue)) = Self::decode_rgb_highlight(highlight) {
            Style::new().fg(Color::Rgb(red, green, blue))
        } else {
            self.highlights[highlight.idx()]
        }
    }

    #[inline]
    pub fn scope(&self, highlight: Highlight) -> &str {
        &self.scopes[highlight.idx()]
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get(&self, scope: &str) -> Style {
        self.try_get(scope).unwrap_or_default()
    }

    /// Get the style of a scope, falling back to dot separated broader
    /// scopes. For example if `ui.text.focus` is not defined in the theme,
    /// `ui.text` is tried and then `ui` is tried.
    pub fn try_get(&self, scope: &str) -> Option<Style> {
        std::iter::successors(Some(scope), |s| Some(s.rsplit_once('.')?.0))
            .find_map(|s| self.styles.get(s).copied())
    }

    /// Get the style of a scope, without falling back to dot separated broader
    /// scopes. For example if `ui.text.focus` is not defined in the theme, it
    /// will return `None`, even if `ui.text` is.
    pub fn try_get_exact(&self, scope: &str) -> Option<Style> {
        self.styles.get(scope).copied()
    }

    /// Returns whether a scope is explicitly defined in this theme
    /// (not just resolved via dot-fallback).
    pub fn has_scope(&self, scope: &str) -> bool {
        self.styles.contains_key(scope)
    }

    /// Classify how a scope resolves in this theme.
    pub fn scope_status(&self, scope: &str) -> ScopeStatus {
        if self.has_scope(scope) {
            ScopeStatus::Defined
        } else if self.try_get(scope).is_some() {
            ScopeStatus::Inherited
        } else {
            ScopeStatus::Missing
        }
    }

    #[inline]
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn find_highlight_exact(&self, scope: &str) -> Option<Highlight> {
        self.scopes()
            .iter()
            .position(|s| s == scope)
            .map(|idx| Highlight::new(idx as u32))
    }

    pub fn find_highlight(&self, mut scope: &str) -> Option<Highlight> {
        loop {
            if let Some(highlight) = self.find_highlight_exact(scope) {
                return Some(highlight);
            }
            if let Some(new_end) = scope.rfind('.') {
                scope = &scope[..new_end];
            } else {
                return None;
            }
        }
    }

    pub fn is_16_color(&self) -> bool {
        self.styles.iter().all(|(_, style)| {
            [style.fg, style.bg]
                .into_iter()
                .all(|color| !matches!(color, Some(Color::Rgb(..))))
        })
    }

    pub fn rainbow_length(&self) -> usize {
        self.rainbow_length
    }

    fn from_toml(value: Value) -> (Self, Vec<String>) {
        if let Value::Table(table) = value {
            Theme::from_keys(table)
        } else {
            warn!("Expected theme TOML value to be a table, found {:?}", value);
            Default::default()
        }
    }

    fn from_keys(toml_keys: Map<String, Value>) -> (Self, Vec<String>) {
        let (styles, scopes, highlights, rainbow_length, load_errors) =
            build_theme_values(toml_keys);

        let theme = Self {
            styles,
            scopes,
            highlights,
            rainbow_length,
            ..Default::default()
        };
        (theme, load_errors)
    }
}

struct ThemePalette {
    palette: HashMap<String, Color>,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self {
            palette: hashmap! {
                "default".to_string() => Color::Reset,
                "black".to_string() => Color::Black,
                "red".to_string() => Color::Red,
                "green".to_string() => Color::Green,
                "yellow".to_string() => Color::Yellow,
                "blue".to_string() => Color::Blue,
                "magenta".to_string() => Color::Magenta,
                "cyan".to_string() => Color::Cyan,
                "gray".to_string() => Color::Gray,
                "light-red".to_string() => Color::LightRed,
                "light-green".to_string() => Color::LightGreen,
                "light-yellow".to_string() => Color::LightYellow,
                "light-blue".to_string() => Color::LightBlue,
                "light-magenta".to_string() => Color::LightMagenta,
                "light-cyan".to_string() => Color::LightCyan,
                "light-gray".to_string() => Color::LightGray,
                "white".to_string() => Color::White,
            },
        }
    }
}

impl ThemePalette {
    pub fn new(palette: HashMap<String, Color>) -> Self {
        let ThemePalette {
            palette: mut default,
        } = ThemePalette::default();

        default.extend(palette);
        Self { palette: default }
    }

    pub fn string_to_rgb(s: &str) -> Result<Color, String> {
        if s.starts_with('#') {
            Color::from_hex(s).map_err(|e| format!("{e}: {s}"))
        } else {
            Self::ansi_string_to_rgb(s)
        }
    }

    fn ansi_string_to_rgb(s: &str) -> Result<Color, String> {
        if let Ok(index) = s.parse::<u8>() {
            return Ok(Color::Indexed(index));
        }
        Err(format!("Malformed ANSI: {}", s))
    }

    fn parse_value_as_str(value: &Value) -> Result<&str, String> {
        value
            .as_str()
            .ok_or(format!("Unrecognized value: {}", value))
    }

    pub fn parse_color(&self, value: Value) -> Result<Color, String> {
        let value = Self::parse_value_as_str(&value)?;

        self.palette
            .get(value)
            .copied()
            .ok_or("")
            .or_else(|_| Self::string_to_rgb(value))
    }

    pub fn parse_modifier(value: &Value) -> Result<Modifier, String> {
        value
            .as_str()
            .and_then(|s| s.parse().ok())
            .ok_or(format!("Invalid modifier: {}", value))
    }

    pub fn parse_underline_style(value: &Value) -> Result<UnderlineStyle, String> {
        value
            .as_str()
            .and_then(|s| s.parse().ok())
            .ok_or(format!("Invalid underline style: {}", value))
    }

    pub fn parse_style(&self, style: &mut Style, value: Value) -> Result<(), String> {
        if let Value::Table(entries) = value {
            for (name, mut value) in entries {
                match name.as_str() {
                    "fg" => *style = style.fg(self.parse_color(value)?),
                    "bg" => *style = style.bg(self.parse_color(value)?),
                    "underline" => {
                        let table = value.as_table_mut().ok_or("Underline must be table")?;
                        if let Some(value) = table.remove("color") {
                            *style = style.underline_color(self.parse_color(value)?);
                        }
                        if let Some(value) = table.remove("style") {
                            *style = style.underline_style(Self::parse_underline_style(&value)?);
                        }

                        if let Some(attr) = table.keys().next() {
                            return Err(format!("Invalid underline attribute: {attr}"));
                        }
                    }
                    "modifiers" => {
                        let modifiers = value.as_array().ok_or("Modifiers should be an array")?;

                        for modifier in modifiers {
                            if modifier.as_str() == Some("underlined") {
                                *style = style.underline_style(UnderlineStyle::Line);
                            } else {
                                *style = style.add_modifier(Self::parse_modifier(modifier)?);
                            }
                        }
                    }
                    _ => return Err(format!("Invalid style attribute: {}", name)),
                }
            }
        } else {
            *style = style.fg(self.parse_color(value)?);
        }
        Ok(())
    }

    fn parse_style_array(&self, value: Value) -> Result<Vec<Style>, String> {
        let mut styles = Vec::new();

        for v in value
            .as_array()
            .ok_or_else(|| format!("Could not parse value as an array: '{value}'"))?
        {
            let mut style = Style::default();
            self.parse_style(&mut style, v.clone())?;
            styles.push(style);
        }

        Ok(styles)
    }
}

impl TryFrom<Value> for ThemePalette {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let map = match value {
            Value::Table(entries) => entries,
            _ => return Ok(Self::default()),
        };

        let mut palette = HashMap::with_capacity(map.len());
        for (name, value) in map {
            let value = Self::parse_value_as_str(&value)?;
            let color = Self::string_to_rgb(value)?;
            palette.insert(name, color);
        }

        Ok(Self::new(palette))
    }
}

/// How a scope resolves in a theme: explicitly defined, inherited via
/// dot-fallback, or completely missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeStatus {
    Defined,
    Inherited,
    Missing,
}

/// Category of a documented theme scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeCategory {
    Syntax,
    Interface,
}

/// Metadata for a single documented theme scope.
#[derive(Debug, Clone, Copy)]
pub struct ScopeInfo {
    /// The scope key, e.g. `"ui.background"` or `"keyword.control.conditional"`.
    pub name: &'static str,
    /// Whether this is a syntax highlight or UI scope.
    pub category: ScopeCategory,
    /// Whether the scope is essential for a complete theme.
    pub essential: bool,
    /// Human-readable description from the themes documentation.
    pub description: &'static str,
    /// Representative sample text for preview rendering.
    pub sample: &'static str,
}

/// Canonical list of all documented theme scopes, sourced from `book/src/themes.md`.
///
/// This is the single source of truth for `:theme-preview`.
pub static DOCUMENTED_SCOPES: &[ScopeInfo] = &[
    ScopeInfo { name: "attribute", category: ScopeCategory::Syntax, essential: false, description: "Class attributes, HTML tag attributes", sample: "#[derive]" },
    ScopeInfo { name: "comment", category: ScopeCategory::Syntax, essential: false, description: "Code comments", sample: "// comment" },
    ScopeInfo { name: "comment.block", category: ScopeCategory::Syntax, essential: false, description: "Block comments", sample: "/* block */" },
    ScopeInfo { name: "comment.block.documentation", category: ScopeCategory::Syntax, essential: false, description: "Block documentation comments", sample: "/** doc */" },
    ScopeInfo { name: "comment.line", category: ScopeCategory::Syntax, essential: false, description: "Single line comments", sample: "// line" },
    ScopeInfo { name: "comment.line.documentation", category: ScopeCategory::Syntax, essential: false, description: "Line documentation comments", sample: "/// doc" },
    ScopeInfo { name: "comment.unused", category: ScopeCategory::Syntax, essential: false, description: "Unused variables and patterns", sample: "_unused" },
    ScopeInfo { name: "constant", category: ScopeCategory::Syntax, essential: false, description: "Constants", sample: "MAX_SIZE" },
    ScopeInfo { name: "constant.builtin", category: ScopeCategory::Syntax, essential: false, description: "Special constants provided by the language", sample: "true nil" },
    ScopeInfo { name: "constant.builtin.boolean", category: ScopeCategory::Syntax, essential: false, description: "Boolean constants", sample: "true false" },
    ScopeInfo { name: "constant.character", category: ScopeCategory::Syntax, essential: false, description: "Character literals", sample: "'a'" },
    ScopeInfo { name: "constant.character.escape", category: ScopeCategory::Syntax, essential: false, description: "Escape sequences", sample: "\\n \\t" },
    ScopeInfo { name: "constant.numeric", category: ScopeCategory::Syntax, essential: false, description: "Numbers", sample: "42 3.14" },
    ScopeInfo { name: "constant.numeric.integer", category: ScopeCategory::Syntax, essential: false, description: "Integer literals", sample: "42" },
    ScopeInfo { name: "constant.numeric.float", category: ScopeCategory::Syntax, essential: false, description: "Floating point literals", sample: "3.14" },
    ScopeInfo { name: "constructor", category: ScopeCategory::Syntax, essential: false, description: "Constructors", sample: "new()" },
    ScopeInfo { name: "hint", category: ScopeCategory::Interface, essential: false, description: "Diagnostics hint (gutter)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "info", category: ScopeCategory::Interface, essential: false, description: "Diagnostics info (gutter)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "warning", category: ScopeCategory::Interface, essential: false, description: "Diagnostics warning (gutter)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "error", category: ScopeCategory::Interface, essential: false, description: "Diagnostics error (gutter)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic", category: ScopeCategory::Interface, essential: false, description: "Diagnostics fallback style (editing area)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic.hint", category: ScopeCategory::Interface, essential: false, description: "Diagnostics hint (editing area)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic.info", category: ScopeCategory::Interface, essential: false, description: "Diagnostics info (editing area)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic.warning", category: ScopeCategory::Interface, essential: false, description: "Diagnostics warning (editing area)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic.error", category: ScopeCategory::Interface, essential: false, description: "Diagnostics error (editing area)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic.unnecessary", category: ScopeCategory::Interface, essential: false, description: "Diagnostics with unnecessary tag", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diagnostic.deprecated", category: ScopeCategory::Interface, essential: false, description: "Diagnostics with deprecated tag", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "diff.delta", category: ScopeCategory::Syntax, essential: false, description: "Modified lines", sample: "~ changed" },
    ScopeInfo { name: "diff.delta.conflict", category: ScopeCategory::Syntax, essential: false, description: "Merge conflicts", sample: "<<<<<<" },
    ScopeInfo { name: "diff.delta.gutter", category: ScopeCategory::Syntax, essential: false, description: "Gutter indicator for modifications", sample: "~" },
    ScopeInfo { name: "diff.delta.moved", category: ScopeCategory::Syntax, essential: false, description: "Renamed or moved changes", sample: "-> moved" },
    ScopeInfo { name: "diff.minus", category: ScopeCategory::Syntax, essential: false, description: "Deleted lines", sample: "- removed" },
    ScopeInfo { name: "diff.minus.gutter", category: ScopeCategory::Syntax, essential: false, description: "Gutter indicator for deletions", sample: "-" },
    ScopeInfo { name: "diff.plus", category: ScopeCategory::Syntax, essential: false, description: "Added lines", sample: "+ added" },
    ScopeInfo { name: "diff.plus.gutter", category: ScopeCategory::Syntax, essential: false, description: "Gutter indicator for additions", sample: "+" },
    ScopeInfo { name: "function", category: ScopeCategory::Syntax, essential: false, description: "Functions", sample: "my_func()" },
    ScopeInfo { name: "function.builtin", category: ScopeCategory::Syntax, essential: false, description: "Built-in functions", sample: "len()" },
    ScopeInfo { name: "function.macro", category: ScopeCategory::Syntax, essential: false, description: "Macros", sample: "println!" },
    ScopeInfo { name: "function.method", category: ScopeCategory::Syntax, essential: false, description: "Methods", sample: ".method()" },
    ScopeInfo { name: "function.method.private", category: ScopeCategory::Syntax, essential: false, description: "Private methods", sample: "#method()" },
    ScopeInfo { name: "function.special", category: ScopeCategory::Syntax, essential: false, description: "Special functions (preprocessor)", sample: "__init__" },
    ScopeInfo { name: "keyword", category: ScopeCategory::Syntax, essential: false, description: "Keywords", sample: "let const" },
    ScopeInfo { name: "keyword.control", category: ScopeCategory::Syntax, essential: false, description: "Control flow keywords", sample: "if for" },
    ScopeInfo { name: "keyword.control.conditional", category: ScopeCategory::Syntax, essential: false, description: "Conditional keywords", sample: "if else" },
    ScopeInfo { name: "keyword.control.exception", category: ScopeCategory::Syntax, essential: false, description: "Exception handling keywords", sample: "try catch" },
    ScopeInfo { name: "keyword.control.import", category: ScopeCategory::Syntax, essential: false, description: "Import keywords", sample: "import use" },
    ScopeInfo { name: "keyword.control.repeat", category: ScopeCategory::Syntax, essential: false, description: "Loop keywords", sample: "for while" },
    ScopeInfo { name: "keyword.control.return", category: ScopeCategory::Syntax, essential: false, description: "Return keywords", sample: "return" },
    ScopeInfo { name: "keyword.directive", category: ScopeCategory::Syntax, essential: false, description: "Preprocessor directives", sample: "#if #define" },
    ScopeInfo { name: "keyword.function", category: ScopeCategory::Syntax, essential: false, description: "Function declaration keywords", sample: "fn func def" },
    ScopeInfo { name: "keyword.operator", category: ScopeCategory::Syntax, essential: false, description: "Operator keywords", sample: "or in" },
    ScopeInfo { name: "keyword.storage", category: ScopeCategory::Syntax, essential: false, description: "Storage keywords", sample: "class var" },
    ScopeInfo { name: "keyword.storage.modifier", category: ScopeCategory::Syntax, essential: false, description: "Storage modifiers", sample: "static mut" },
    ScopeInfo { name: "keyword.storage.type", category: ScopeCategory::Syntax, essential: false, description: "Type storage keywords", sample: "class let" },
    ScopeInfo { name: "label", category: ScopeCategory::Syntax, essential: false, description: "Labels", sample: "'label:" },
    ScopeInfo { name: "markup.bold", category: ScopeCategory::Syntax, essential: false, description: "Bold text", sample: "**bold**" },
    ScopeInfo { name: "markup.italic", category: ScopeCategory::Syntax, essential: false, description: "Italic text", sample: "*italic*" },
    ScopeInfo { name: "markup.heading", category: ScopeCategory::Syntax, essential: false, description: "Headings", sample: "# Heading" },
    ScopeInfo { name: "markup.heading.1", category: ScopeCategory::Syntax, essential: false, description: "Heading level 1", sample: "# H1" },
    ScopeInfo { name: "markup.heading.2", category: ScopeCategory::Syntax, essential: false, description: "Heading level 2", sample: "## H2" },
    ScopeInfo { name: "markup.heading.3", category: ScopeCategory::Syntax, essential: false, description: "Heading level 3", sample: "### H3" },
    ScopeInfo { name: "markup.heading.4", category: ScopeCategory::Syntax, essential: false, description: "Heading level 4", sample: "#### H4" },
    ScopeInfo { name: "markup.heading.5", category: ScopeCategory::Syntax, essential: false, description: "Heading level 5", sample: "##### H5" },
    ScopeInfo { name: "markup.heading.6", category: ScopeCategory::Syntax, essential: false, description: "Heading level 6", sample: "###### H6" },
    ScopeInfo { name: "markup.heading.completion", category: ScopeCategory::Interface, essential: false, description: "Completion doc heading", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "markup.heading.hover", category: ScopeCategory::Interface, essential: false, description: "Hover heading", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "markup.heading.marker", category: ScopeCategory::Syntax, essential: false, description: "Heading markers", sample: "#" },
    ScopeInfo { name: "markup.link", category: ScopeCategory::Syntax, essential: false, description: "Links", sample: "[text](url)" },
    ScopeInfo { name: "markup.link.label", category: ScopeCategory::Syntax, essential: false, description: "Non-URL link references", sample: "[ref]" },
    ScopeInfo { name: "markup.link.text", category: ScopeCategory::Syntax, essential: false, description: "URL and image descriptions", sample: "[description]" },
    ScopeInfo { name: "markup.link.url", category: ScopeCategory::Syntax, essential: false, description: "URLs pointed to by links", sample: "https://..." },
    ScopeInfo { name: "markup.list", category: ScopeCategory::Syntax, essential: false, description: "Lists", sample: "- item" },
    ScopeInfo { name: "markup.list.numbered", category: ScopeCategory::Syntax, essential: false, description: "Numbered lists", sample: "1. item" },
    ScopeInfo { name: "markup.list.checked", category: ScopeCategory::Syntax, essential: false, description: "Checked items", sample: "[x] done" },
    ScopeInfo { name: "markup.list.unchecked", category: ScopeCategory::Syntax, essential: false, description: "Unchecked items", sample: "[ ] todo" },
    ScopeInfo { name: "markup.list.unnumbered", category: ScopeCategory::Syntax, essential: false, description: "Bullet lists", sample: "- item" },
    ScopeInfo { name: "markup.normal.completion", category: ScopeCategory::Interface, essential: false, description: "Completion doc popup", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "markup.normal.hover", category: ScopeCategory::Interface, essential: false, description: "Hover popup", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "markup.quote", category: ScopeCategory::Syntax, essential: false, description: "Block quotes", sample: "> quote" },
    ScopeInfo { name: "markup.raw", category: ScopeCategory::Syntax, essential: false, description: "Raw/code blocks", sample: "`code`" },
    ScopeInfo { name: "markup.raw.block", category: ScopeCategory::Syntax, essential: false, description: "Code blocks", sample: "```block```" },
    ScopeInfo { name: "markup.raw.inline", category: ScopeCategory::Syntax, essential: false, description: "Inline code", sample: "`inline`" },
    ScopeInfo { name: "markup.raw.inline.completion", category: ScopeCategory::Interface, essential: false, description: "Inline code in completion doc", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "markup.raw.inline.hover", category: ScopeCategory::Interface, essential: false, description: "Inline code in hover", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "markup.strikethrough", category: ScopeCategory::Syntax, essential: false, description: "Strikethrough text", sample: "~~struck~~" },
    ScopeInfo { name: "namespace", category: ScopeCategory::Syntax, essential: false, description: "Namespaces/modules", sample: "std::io" },
    ScopeInfo { name: "operator", category: ScopeCategory::Syntax, essential: false, description: "Operators", sample: "|| += >" },
    ScopeInfo { name: "punctuation", category: ScopeCategory::Syntax, essential: false, description: "Punctuation", sample: ". ; :" },
    ScopeInfo { name: "punctuation.bracket", category: ScopeCategory::Syntax, essential: false, description: "Parentheses, angle brackets", sample: "( ) { }" },
    ScopeInfo { name: "punctuation.delimiter", category: ScopeCategory::Syntax, essential: false, description: "Commas, colons", sample: ", :" },
    ScopeInfo { name: "punctuation.special", category: ScopeCategory::Syntax, essential: false, description: "String interpolation brackets", sample: "${}" },
    ScopeInfo { name: "special", category: ScopeCategory::Syntax, essential: false, description: "Derive declarations, bolded matches", sample: "#[derive]" },
    ScopeInfo { name: "string", category: ScopeCategory::Syntax, essential: false, description: "String literals", sample: "\"hello world\"" },
    ScopeInfo { name: "string.regexp", category: ScopeCategory::Syntax, essential: false, description: "Regular expressions", sample: "/pattern/" },
    ScopeInfo { name: "string.special", category: ScopeCategory::Syntax, essential: false, description: "Special strings", sample: "special" },
    ScopeInfo { name: "string.special.path", category: ScopeCategory::Syntax, essential: false, description: "File paths", sample: "/usr/bin" },
    ScopeInfo { name: "string.special.symbol", category: ScopeCategory::Syntax, essential: false, description: "Symbols (atoms, Ruby symbols)", sample: ":ok" },
    ScopeInfo { name: "string.special.url", category: ScopeCategory::Syntax, essential: false, description: "URLs", sample: "https://..." },
    ScopeInfo { name: "tabstop", category: ScopeCategory::Interface, essential: false, description: "Snippet placeholder", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "tag", category: ScopeCategory::Syntax, essential: false, description: "Tags (e.g. HTML tags)", sample: "<div>" },
    ScopeInfo { name: "tag.builtin", category: ScopeCategory::Syntax, essential: false, description: "Built-in tags", sample: "<body>" },
    ScopeInfo { name: "type", category: ScopeCategory::Syntax, essential: false, description: "Types", sample: "String" },
    ScopeInfo { name: "type.builtin", category: ScopeCategory::Syntax, essential: false, description: "Primitive types provided by the language", sample: "int usize" },
    ScopeInfo { name: "type.enum", category: ScopeCategory::Syntax, essential: false, description: "Enum types", sample: "Option" },
    ScopeInfo { name: "type.enum.variant", category: ScopeCategory::Syntax, essential: false, description: "Enum variants", sample: "Some None" },
    ScopeInfo { name: "type.parameter", category: ScopeCategory::Syntax, essential: false, description: "Generic type parameters", sample: "T" },
    ScopeInfo { name: "variable", category: ScopeCategory::Syntax, essential: false, description: "Variables", sample: "my_var" },
    ScopeInfo { name: "variable.builtin", category: ScopeCategory::Syntax, essential: false, description: "Reserved language variables", sample: "self this" },
    ScopeInfo { name: "variable.builtin.mutable", category: ScopeCategory::Syntax, essential: false, description: "Mutable language variables", sample: "mut self" },
    ScopeInfo { name: "variable.mutable", category: ScopeCategory::Syntax, essential: false, description: "Mutable variables", sample: "mut x" },
    ScopeInfo { name: "variable.parameter", category: ScopeCategory::Syntax, essential: false, description: "Function parameters", sample: "arg param" },
    ScopeInfo { name: "variable.parameter.mutable", category: ScopeCategory::Syntax, essential: false, description: "Mutable function parameters", sample: "mut arg" },
    ScopeInfo { name: "variable.other", category: ScopeCategory::Syntax, essential: false, description: "Other variables", sample: "other" },
    ScopeInfo { name: "variable.other.member", category: ScopeCategory::Syntax, essential: false, description: "Fields of composite data types", sample: "self.field" },
    ScopeInfo { name: "variable.other.member.private", category: ScopeCategory::Syntax, essential: false, description: "Private fields", sample: "#private" },
    ScopeInfo { name: "ui.background", category: ScopeCategory::Interface, essential: false, description: "Editor background", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.background.separator", category: ScopeCategory::Interface, essential: false, description: "Picker separator below input line", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.bufferline", category: ScopeCategory::Interface, essential: false, description: "Buffer line tabs", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.bufferline.active", category: ScopeCategory::Interface, essential: false, description: "Active buffer tab", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.bufferline.background", category: ScopeCategory::Interface, essential: false, description: "Bufferline background", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor", category: ScopeCategory::Interface, essential: false, description: "Cursor", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.insert", category: ScopeCategory::Interface, essential: false, description: "Insert mode cursor", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.match", category: ScopeCategory::Interface, essential: false, description: "Matching bracket indicator", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.normal", category: ScopeCategory::Interface, essential: false, description: "Normal mode cursor", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.primary", category: ScopeCategory::Interface, essential: false, description: "Primary cursor with multiple selections", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.primary.insert", category: ScopeCategory::Interface, essential: false, description: "Primary cursor in insert mode", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.primary.normal", category: ScopeCategory::Interface, essential: false, description: "Primary cursor in normal mode", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.primary.select", category: ScopeCategory::Interface, essential: false, description: "Primary cursor in select mode", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursor.select", category: ScopeCategory::Interface, essential: false, description: "Select mode cursor", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursorcolumn.primary", category: ScopeCategory::Interface, essential: false, description: "Column of primary cursor", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursorcolumn.secondary", category: ScopeCategory::Interface, essential: false, description: "Columns of other cursors", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursorline.primary", category: ScopeCategory::Interface, essential: false, description: "Line of primary cursor", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.cursorline.secondary", category: ScopeCategory::Interface, essential: false, description: "Lines of other cursors", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.debug.active", category: ScopeCategory::Interface, essential: false, description: "Debug execution paused indicator", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.debug.breakpoint", category: ScopeCategory::Interface, essential: false, description: "Breakpoint indicator in gutter", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.gutter", category: ScopeCategory::Interface, essential: false, description: "Gutter", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.gutter.selected", category: ScopeCategory::Interface, essential: false, description: "Gutter for the cursor line", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.help", category: ScopeCategory::Interface, essential: false, description: "Description box for commands", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.highlight", category: ScopeCategory::Interface, essential: false, description: "Highlighted lines in picker preview", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.highlight.frameline", category: ScopeCategory::Interface, essential: false, description: "Line at which debugging execution is paused", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.linenr", category: ScopeCategory::Interface, essential: false, description: "Line numbers", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.linenr.selected", category: ScopeCategory::Interface, essential: false, description: "Line number for cursor line", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.menu", category: ScopeCategory::Interface, essential: false, description: "Code and command completion menus", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.menu.scroll", category: ScopeCategory::Interface, essential: false, description: "Scrollbar (fg=thumb, bg=track)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.menu.selected", category: ScopeCategory::Interface, essential: false, description: "Selected autocomplete item", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.picker.header", category: ScopeCategory::Interface, essential: false, description: "Header row in pickers", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.picker.header.column", category: ScopeCategory::Interface, essential: false, description: "Column names in pickers", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.picker.header.column.active", category: ScopeCategory::Interface, essential: false, description: "Active column name in picker", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.popup", category: ScopeCategory::Interface, essential: false, description: "Documentation popups", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.popup.info", category: ScopeCategory::Interface, essential: false, description: "Prompt for multiple key options", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.selection", category: ScopeCategory::Interface, essential: true, description: "Selections in editing area", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.selection.primary", category: ScopeCategory::Interface, essential: false, description: "Primary selection", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.statusline", category: ScopeCategory::Interface, essential: false, description: "Statusline", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.statusline.inactive", category: ScopeCategory::Interface, essential: false, description: "Statusline (unfocused document)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.statusline.insert", category: ScopeCategory::Interface, essential: false, description: "Statusline in insert mode", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.statusline.normal", category: ScopeCategory::Interface, essential: false, description: "Statusline in normal mode", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.statusline.select", category: ScopeCategory::Interface, essential: false, description: "Statusline in select mode", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.statusline.separator", category: ScopeCategory::Interface, essential: false, description: "Separator character in statusline", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.text", category: ScopeCategory::Interface, essential: false, description: "Default text style, command prompts, popup text", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.text.directory", category: ScopeCategory::Interface, essential: false, description: "Directory names in prompt completion", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.text.focus", category: ScopeCategory::Interface, essential: false, description: "Currently selected line in picker", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.text.inactive", category: ScopeCategory::Interface, essential: false, description: "Text when inactive (e.g. suggestions)", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.text.info", category: ScopeCategory::Interface, essential: false, description: "Key text in popup.info boxes", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.text.symlink", category: ScopeCategory::Interface, essential: false, description: "Symlink names in prompt completion", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.indent-guide", category: ScopeCategory::Interface, essential: false, description: "Vertical indent width guides", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.inlay-hint", category: ScopeCategory::Interface, essential: false, description: "Default style for inlay hints", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.inlay-hint.parameter", category: ScopeCategory::Interface, essential: false, description: "Parameter inlay hints", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.inlay-hint.type", category: ScopeCategory::Interface, essential: false, description: "Type inlay hints", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.jump-label", category: ScopeCategory::Interface, essential: false, description: "Virtual jump labels", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.ruler", category: ScopeCategory::Interface, essential: false, description: "Ruler columns", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.whitespace", category: ScopeCategory::Interface, essential: false, description: "Visible whitespace characters", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.virtual.wrap", category: ScopeCategory::Interface, essential: false, description: "Soft-wrap indicator", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
    ScopeInfo { name: "ui.window", category: ScopeCategory::Interface, essential: false, description: "Borderlines separating splits", sample: "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}" },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_style_string() {
        let fg = Value::String("#ffffff".to_string());

        let mut style = Style::default();
        let palette = ThemePalette::default();
        palette.parse_style(&mut style, fg).unwrap();

        assert_eq!(style, Style::default().fg(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn test_palette() {
        use helix_core::hashmap;
        let fg = Value::String("my_color".to_string());

        let mut style = Style::default();
        let palette =
            ThemePalette::new(hashmap! { "my_color".to_string() => Color::Rgb(255, 255, 255) });
        palette.parse_style(&mut style, fg).unwrap();

        assert_eq!(style, Style::default().fg(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn test_parse_style_table() {
        let table = toml::toml! {
            "keyword" = {
                fg = "#ffffff",
                bg = "#000000",
                modifiers = ["bold"],
            }
        };

        let mut style = Style::default();
        let palette = ThemePalette::default();
        for (_name, value) in table {
            palette.parse_style(&mut style, value).unwrap();
        }

        assert_eq!(
            style,
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .bg(Color::Rgb(0, 0, 0))
                .add_modifier(Modifier::BOLD)
        );
    }

    // tests for parsing an RGB `Highlight`

    #[test]
    fn convert_to_and_from() {
        let (r, g, b) = (0xFF, 0xFE, 0xFA);
        let highlight = Theme::rgb_highlight(r, g, b);
        assert_eq!(Theme::decode_rgb_highlight(highlight), Some((r, g, b)));
    }

    /// make sure we can store all the colors at the end
    #[test]
    fn full_numeric_range() {
        assert_eq!(Highlight::MAX - Theme::RGB_START, 256_u32.pow(3));
    }

    #[test]
    fn retrieve_color() {
        // color in the middle
        let (r, g, b) = (0x14, 0xAA, 0xF7);
        assert_eq!(
            Theme::default().highlight(Theme::rgb_highlight(r, g, b)),
            Style::new().fg(Color::Rgb(r, g, b))
        );
        // pure black
        let (r, g, b) = (0x00, 0x00, 0x00);
        assert_eq!(
            Theme::default().highlight(Theme::rgb_highlight(r, g, b)),
            Style::new().fg(Color::Rgb(r, g, b))
        );
        // pure white
        let (r, g, b) = (0xff, 0xff, 0xff);
        assert_eq!(
            Theme::default().highlight(Theme::rgb_highlight(r, g, b)),
            Style::new().fg(Color::Rgb(r, g, b))
        );
    }

    #[test]
    #[should_panic(expected = "index out of bounds: the len is 0 but the index is 4278190078")]
    fn out_of_bounds() {
        let highlight = Highlight::new(Theme::rgb_highlight(0, 0, 0).get() - 1);
        Theme::default().highlight(highlight);
    }

    #[test]
    fn documented_scopes_not_empty() {
        assert!(
            !DOCUMENTED_SCOPES.is_empty(),
            "DOCUMENTED_SCOPES must contain entries"
        );
    }

    #[test]
    fn documented_scopes_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for scope in DOCUMENTED_SCOPES {
            assert!(
                seen.insert(scope.name),
                "Duplicate scope name in DOCUMENTED_SCOPES: {}",
                scope.name
            );
        }
    }

    #[test]
    fn default_theme_has_essential_scopes() {
        let theme = &*DEFAULT_THEME;
        let missing: Vec<&str> = DOCUMENTED_SCOPES
            .iter()
            .filter(|s| s.essential)
            .filter(|s| theme.try_get(s.name).is_none())
            .map(|s| s.name)
            .collect();
        assert!(
            missing.is_empty(),
            "Default theme is missing essential scopes: {:?}",
            missing
        );
    }
}
