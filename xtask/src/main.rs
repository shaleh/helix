mod docgen;
mod helpers;
mod path;

use std::{env, error::Error};

type DynError = Box<dyn Error>;

pub mod tasks {
    use crate::DynError;
    use std::collections::HashSet;

    pub fn docgen() -> Result<(), DynError> {
        use crate::docgen::*;
        write(TYPABLE_COMMANDS_MD_OUTPUT, &typable_commands()?);
        write(STATIC_COMMANDS_MD_OUTPUT, &static_commands()?);
        write(LANG_SUPPORT_MD_OUTPUT, &lang_features()?);
        Ok(())
    }

    pub fn querycheck(languages: impl Iterator<Item = String>) -> Result<(), DynError> {
        use helix_core::syntax::LanguageData;

        let languages_to_check: HashSet<_> = languages.collect();
        let loader = helix_core::config::default_lang_loader();
        for (_language, lang_data) in loader.languages() {
            if !languages_to_check.is_empty()
                && !languages_to_check.contains(&lang_data.config().language_id)
            {
                continue;
            }
            let config = lang_data.config();
            let Some(syntax_config) = LanguageData::compile_syntax_config(config, &loader)? else {
                continue;
            };
            let grammar = syntax_config.grammar;
            LanguageData::compile_indent_query(grammar, config)?;
            LanguageData::compile_textobject_query(grammar, config)?;
            LanguageData::compile_tag_query(grammar, config)?;
            LanguageData::compile_rainbow_query(grammar, config)?;
        }

        println!("Query check succeeded");

        Ok(())
    }

    pub fn themecheck(themes: impl Iterator<Item = String>) -> Result<(), DynError> {
        use helix_view::theme::{Loader, DOCUMENTED_SCOPES};

        let themes_to_check: HashSet<_> = themes.collect();

        let theme_names = [
            vec!["default".to_string(), "base16_default".to_string()],
            Loader::read_names(&crate::path::themes()),
        ]
        .concat();
        let loader = Loader::new(&[crate::path::runtime()]);
        let mut errors_present = false;

        struct ThemeReport {
            name: String,
            covered: usize,
            total: usize,
            load_errors: Vec<String>,
            missing_essential: Vec<&'static str>,
        }

        let mut reports = Vec::new();

        for name in theme_names {
            if !themes_to_check.is_empty() && !themes_to_check.contains(&name) {
                continue;
            }

            let (theme, warnings) = loader.load_with_warnings(&name).unwrap();

            if !warnings.is_empty() {
                errors_present = true;
            }

            // Check scope coverage — use try_get (with fallback) so
            // inherited and dot-fallback scopes count as covered.
            let mut missing_essential = Vec::new();
            let mut missing_count = 0usize;

            for scope in DOCUMENTED_SCOPES {
                if theme.try_get(scope.name).is_none() {
                    missing_count += 1;
                    if scope.essential {
                        missing_essential.push(scope.name);
                    }
                }
            }

            if !missing_essential.is_empty() {
                errors_present = true;
            }

            reports.push(ThemeReport {
                name,
                covered: DOCUMENTED_SCOPES.len() - missing_count,
                total: DOCUMENTED_SCOPES.len(),
                load_errors: warnings,
                missing_essential,
            });
        }

        // Sort by coverage descending, then name ascending.
        reports.sort_by(|a, b| b.covered.cmp(&a.covered).then(a.name.cmp(&b.name)));

        for report in &reports {
            if !report.load_errors.is_empty() {
                println!("Theme '{}' loaded with errors:", report.name);
                for warning in &report.load_errors {
                    println!("\t* {warning}");
                }
            }

            if !report.missing_essential.is_empty() {
                println!("Theme '{}' is missing essential scopes:", report.name);
                for scope in &report.missing_essential {
                    println!("\t! {scope}");
                }
            }

            let missing_optional = report.total - report.covered - report.missing_essential.len();
            println!(
                "{:>3}/{} {} ({} optional missing)",
                report.covered, report.total, report.name, missing_optional,
            );
        }

        match errors_present {
            true => Err("Errors found when loading bundled themes".into()),
            false => {
                println!("Theme check successful!");
                Ok(())
            }
        }
    }

    pub fn theme_check_accessibility(
        themes: impl Iterator<Item = String>,
    ) -> Result<(), DynError> {
        use helix_view::accessibility::AccessibilityReport;
        use helix_view::theme::Loader;

        let themes_to_check: HashSet<_> = themes.collect();

        let theme_names = [
            vec!["default".to_string(), "base16_default".to_string()],
            Loader::read_names(&crate::path::themes()),
        ]
        .concat();
        let loader = Loader::new(&[crate::path::runtime()]);

        struct A11yReport {
            name: String,
            aa_percent: f64,
            aaa_percent: f64,
            pass: usize,
            fail: usize,
            unknown: usize,
        }

        let mut reports = Vec::new();

        for name in theme_names {
            if !themes_to_check.is_empty() && !themes_to_check.contains(&name) {
                continue;
            }

            let (theme, _) = loader.load_with_warnings(&name).unwrap();
            let a11y = AccessibilityReport::analyze(&theme);

            reports.push(A11yReport {
                name,
                aa_percent: a11y.aa_percent(),
                aaa_percent: a11y.aaa_percent(),
                pass: a11y.pass_aaa + a11y.pass_aa,
                fail: a11y.fail,
                unknown: a11y.unknown,
            });
        }

        // Sort by AA percentage descending, then name ascending.
        reports.sort_by(|a, b| {
            b.aa_percent
                .partial_cmp(&a.aa_percent)
                .unwrap()
                .then(a.name.cmp(&b.name))
        });

        for report in &reports {
            println!(
                "{:>3}% AA  {:>3}% AAA  {} ({} pass, {} fail, {} unknown)",
                report.aa_percent as u32,
                report.aaa_percent as u32,
                report.name,
                report.pass,
                report.fail,
                report.unknown,
            );
        }

        println!("Accessibility check complete.");
        Ok(())
    }

    pub fn theme_analyze(mut args: impl Iterator<Item = String>) -> Result<(), DynError> {
        use helix_view::accessibility::{
            ansi_colored_text, contrast_ratio, resolve_color, rgb_to_hex, suggest_fixes,
            AccessibilityReport, Readability, WcagLevel,
        };
        use helix_view::theme::Loader;

        let theme_name = args
            .next()
            .ok_or("Usage: cargo xtask theme-analyze <theme-name>")?;

        let loader = Loader::new(&[crate::path::runtime()]);
        let (theme, _) = loader
            .load_with_warnings(&theme_name)
            .map_err(|e| format!("Could not load theme '{}': {}", theme_name, e))?;

        let report = AccessibilityReport::analyze(&theme);

        // Load the fully merged TOML (with inheritance resolved) for palette analysis.
        let merged_toml: Option<toml::Value> = loader.load_merged_toml(&theme_name).ok();

        // Build palette map: palette_name → hex_color
        let palette: std::collections::HashMap<String, String> = merged_toml
            .as_ref()
            .and_then(|v| v.get("palette"))
            .and_then(|p| p.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // Build reverse map: scope_name → palette_color_name (for scopes that reference palette)
        let scope_to_palette: std::collections::HashMap<String, String> = merged_toml
            .as_ref()
            .map(|root| {
                let mut map = std::collections::HashMap::new();
                if let Some(table) = root.as_table() {
                    for (key, value) in table {
                        if key == "palette" || key == "inherits" {
                            continue;
                        }
                        // Extract fg color reference
                        let fg_ref = match value {
                            toml::Value::String(s) => Some(s.clone()),
                            toml::Value::Table(t) => {
                                t.get("fg").and_then(|v| v.as_str()).map(|s| s.to_string())
                            }
                            _ => None,
                        };
                        if let Some(color_name) = fg_ref {
                            if palette.contains_key(&color_name) {
                                map.insert(key.clone(), color_name);
                            }
                        }
                    }
                }
                map
            })
            .unwrap_or_default();

        // ── Header ──
        let computable = report.computable();

        println!();
        println!("  Theme: {theme_name}");
        println!();

        if computable > 0 {
            let readability = if report.aa_percent() >= 90.0 {
                "easy to read"
            } else if report.aa_percent() >= 70.0 {
                "mostly readable"
            } else if report.aa_percent() >= 50.0 {
                "needs work"
            } else {
                "significant accessibility issues"
            };
            println!(
                "  Overall: {} ({:.0}% of text meets minimum standard)",
                readability,
                report.aa_percent()
            );
        } else {
            println!(
                "  Overall: cannot assess (theme uses terminal-dependent colors)"
            );
        }
        println!();

        // Resolve effective bg for the theme
        let bg_color = theme.try_get("ui.background").and_then(|s| s.bg);

        // ── Needs attention ──
        let failing: Vec<_> = report
            .scopes
            .iter()
            .filter(|s| matches!(s.result.level(), Some(WcagLevel::Fail)))
            .collect();

        if failing.is_empty() {
            println!("  \x1b[32m✓\x1b[0m No accessibility issues found!");
            println!();
        } else {
            println!(
                "  ─── Needs attention ({} scopes) ───",
                failing.len()
            );
            println!();
            for scope in &failing {
                let (fg, bg) = helix_view::accessibility::resolve_contrast_pair(&theme, scope.name);
                let fg_rgb = fg.and_then(|c| resolve_color(c).rgb());
                let bg_rgb_resolved = bg.and_then(|c| resolve_color(c).rgb());

                if let (Some(fg_rgb), Some(bg_rgb)) = (fg_rgb, bg_rgb_resolved) {
                    let ratio = contrast_ratio(fg_rgb, bg_rgb);
                    let readability = Readability::from_ratio(ratio);

                    // Show the scope with actual colors
                    let preview = ansi_colored_text(
                        &format!(" {} ", scope.name),
                        fg_rgb,
                        bg_rgb,
                    );
                    println!(
                        "    {} {} — {} ({:.1}:1)",
                        preview,
                        rgb_to_hex(fg_rgb.0, fg_rgb.1, fg_rgb.2),
                        readability.label(),
                        ratio,
                    );
                    println!("      {}", readability.description());

                    // Suggestions
                    use helix_view::accessibility::AdjustTarget;
                    let suggestions = suggest_fixes(fg_rgb, bg_rgb);

                    // Group by target level, show fg then bg option for each
                    for target_level in &[WcagLevel::AA, WcagLevel::AAA] {
                        let level_label = match target_level {
                            WcagLevel::AA => "readable",
                            WcagLevel::AAA => "easy to read",
                            WcagLevel::Fail => continue,
                        };

                        let level_suggestions: Vec<_> = suggestions
                            .iter()
                            .filter(|s| &s.target_level == target_level)
                            .collect();

                        for s in &level_suggestions {
                            let (what, preview_fg, preview_bg) = match s.target {
                                AdjustTarget::Foreground => (
                                    format!("fg → {}", rgb_to_hex(s.suggested.0, s.suggested.1, s.suggested.2)),
                                    s.suggested,
                                    bg_rgb,
                                ),
                                AdjustTarget::Background => (
                                    format!("bg → {}", rgb_to_hex(s.suggested.0, s.suggested.1, s.suggested.2)),
                                    fg_rgb,
                                    s.suggested,
                                ),
                            };
                            let preview = ansi_colored_text(
                                &format!(" {} ", scope.name),
                                preview_fg,
                                preview_bg,
                            );
                            println!(
                                "      {}: {} {} — {} ({:.1}:1)",
                                what, preview,
                                rgb_to_hex(preview_fg.0, preview_fg.1, preview_fg.2),
                                level_label,
                                s.suggested_ratio,
                            );
                        }
                    }
                    println!();
                }
            }
        }

        // ── Palette opportunities ──
        if !scope_to_palette.is_empty() && !failing.is_empty() {
            // Group failing scopes by their palette color
            let mut palette_failures: std::collections::HashMap<String, Vec<&str>> =
                std::collections::HashMap::new();

            for scope in &failing {
                if let Some(palette_name) = scope_to_palette.get(scope.name) {
                    palette_failures
                        .entry(palette_name.clone())
                        .or_default()
                        .push(scope.name);
                }
            }

            // Only show palette entries that affect multiple scopes
            let multi_scope_entries: Vec<_> = palette_failures
                .iter()
                .filter(|(_, scopes)| scopes.len() >= 2)
                .collect();

            if !multi_scope_entries.is_empty() {
                println!("  ─── Palette opportunities ───");
                println!();
                println!(
                    "  Changing these palette colors would fix multiple scopes at once:"
                );
                println!();

                let bg_rgb = bg_color
                    .and_then(|c| resolve_color(c).rgb())
                    .unwrap_or((0, 0, 0));

                for (palette_name, scopes) in &multi_scope_entries {
                    if let Some(hex) = palette.get(*palette_name) {
                        // Try to parse the palette hex to RGB
                        if let Some(rgb) = parse_hex(hex) {
                            let suggestions = suggest_fixes(rgb, bg_rgb);
                            if let Some(aa_fix) = suggestions.first() {
                                println!(
                                    "    {} = \"{}\" → \"{}\"",
                                    palette_name,
                                    hex,
                                    rgb_to_hex(
                                        aa_fix.suggested.0,
                                        aa_fix.suggested.1,
                                        aa_fix.suggested.2
                                    ),
                                );
                                println!(
                                    "      fixes: {}",
                                    scopes.join(", ")
                                );
                                println!();
                            }
                        }
                    }
                }
            }
        }

        // ── Unknown scopes ──
        let unknowns: Vec<_> = report
            .scopes
            .iter()
            .filter(|s| matches!(s.result, helix_view::accessibility::ContrastResult::Unknown))
            .collect();

        if !unknowns.is_empty() {
            use helix_view::accessibility::unknown_reason;

            // Group by reason
            let mut by_reason: std::collections::HashMap<&str, Vec<&str>> =
                std::collections::HashMap::new();
            for scope in &unknowns {
                let reason = unknown_reason(&theme, scope.name);
                by_reason
                    .entry(reason.label())
                    .or_default()
                    .push(scope.name);
            }

            println!(
                "  ─── Unknown ({} scopes) ───",
                unknowns.len()
            );
            println!();
            println!(
                "  These scopes could not be checked because their colors are not"
            );
            println!(
                "  fully specified. Adding explicit hex colors makes them assessable."
            );
            println!();

            for (reason, scopes) in &by_reason {
                let recommendation = unknowns
                    .iter()
                    .find(|s| scopes.contains(&s.name))
                    .map(|s| unknown_reason(&theme, s.name).recommendation())
                    .unwrap_or("");

                println!("    {reason} ({} scopes):", scopes.len());
                println!("      Recommendation: {recommendation}");
                // Show first few scope names
                let display_count = scopes.len().min(5);
                let scope_list = scopes[..display_count].join(", ");
                if scopes.len() > 5 {
                    println!(
                        "      Scopes: {scope_list}, ... and {} more",
                        scopes.len() - 5
                    );
                } else {
                    println!("      Scopes: {scope_list}");
                }
                println!();
            }
        }

        // ── Acknowledged scopes ──
        let acknowledged: Vec<&str> = merged_toml
            .as_ref()
            .and_then(|v| v.get("accessibility"))
            .and_then(|a| a.get("acknowledged"))
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect()
            })
            .unwrap_or_default();

        if !acknowledged.is_empty() {
            println!("  ─── Acknowledged ({} scopes) ───", acknowledged.len());
            println!();
            println!("  These were marked as intentional stylistic choices:");
            for scope in &acknowledged {
                println!("    {scope}");
            }
            println!();
        }

        // ── Summary ──
        println!("  ─── Summary ───");
        println!();
        println!(
            "    Easy to read (AAA):   {:>3} scopes",
            report.pass_aaa
        );
        println!(
            "    Readable (AA):        {:>3} scopes",
            report.pass_aa
        );
        println!(
            "    Hard to read:         {:>3} scopes  {}",
            report.fail,
            if report.fail > 0 { "← fix these" } else { "" }
        );
        if report.unknown > 0 {
            println!(
                "    Cannot assess:        {:>3} scopes  (no explicit colors set)",
                report.unknown
            );
        }
        println!();

        Ok(())
    }

    fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
        let s = s.strip_prefix('#')?;
        match s.len() {
            6 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some((r, g, b))
            }
            3 => {
                let r = u8::from_str_radix(&s[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&s[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&s[2..3], 16).ok()? * 17;
                Some((r, g, b))
            }
            _ => None,
        }
    }

    pub fn print_help() {
        println!(
            "
Usage: Run with `cargo xtask <task>`, eg. `cargo xtask docgen`.

    Tasks:
        docgen                     Generate files to be included in the mdbook output.
        query-check [languages]    Check that tree-sitter queries are valid for the given
                                   languages, or all languages if none are specified.
        theme-check [themes]       Check that the theme files in runtime/themes/ are valid for the
                                   given themes, or all themes if none are specified.
        theme-check-accessibility [themes]
                                   Check WCAG color contrast accessibility for themes,
                                   sorted by AA compliance percentage.
        theme-analyze <theme>      Detailed accessibility analysis for a single theme
                                   with readability descriptions and color suggestions.
"
        );
    }
}

fn main() -> Result<(), DynError> {
    let mut args = env::args().skip(1);
    let task = args.next();
    match task {
        None => tasks::print_help(),
        Some(t) => match t.as_str() {
            "docgen" => tasks::docgen()?,
            "query-check" => tasks::querycheck(args)?,
            "theme-check" => tasks::themecheck(args)?,
            "theme-check-accessibility" => tasks::theme_check_accessibility(args)?,
            "theme-analyze" => tasks::theme_analyze(args)?,
            invalid => return Err(format!("Invalid task name: {}", invalid).into()),
        },
    };
    Ok(())
}
