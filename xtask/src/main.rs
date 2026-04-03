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

    pub fn theme_fix(mut args: impl Iterator<Item = String>) -> Result<(), DynError> {
        use helix_view::accessibility::{
            ansi_colored_text, contrast_ratio, resolve_color, rgb_to_hex, suggest_fixes,
            AccessibilityReport, AdjustTarget, Readability, WcagLevel,
        };
        use helix_view::theme::Loader;
        use std::io::{self, BufRead, Write};

        let theme_name = args
            .next()
            .ok_or("Usage: cargo xtask theme-fix <theme-name>")?;

        let loader = Loader::new(&[crate::path::runtime()]);
        let (theme, _) = loader
            .load_with_warnings(&theme_name)
            .map_err(|e| format!("Could not load theme '{}': {}", theme_name, e))?;

        let report = AccessibilityReport::analyze(&theme);
        let merged_toml: Option<toml::Value> = loader.load_merged_toml(&theme_name).ok();

        // Build palette map
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

        // scope → palette fg name
        let scope_to_palette: std::collections::HashMap<String, String> = merged_toml
            .as_ref()
            .map(|root| {
                let mut map = std::collections::HashMap::new();
                if let Some(table) = root.as_table() {
                    for (key, value) in table {
                        if key == "palette" || key == "inherits" || key == "accessibility" {
                            continue;
                        }
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

        let failing: Vec<_> = report
            .scopes
            .iter()
            .filter(|s| matches!(s.result.level(), Some(WcagLevel::Fail)))
            .collect();

        if failing.is_empty() {
            println!("\n  \x1b[32m✓\x1b[0m Theme '{}' has no accessibility issues!\n", theme_name);
            return Ok(());
        }

        // Resolve effective bg for the theme
        let theme_bg = theme.try_get("ui.background").and_then(|s| s.bg);
        let theme_bg_rgb = theme_bg.and_then(|c| resolve_color(c).rgb()).unwrap_or((0, 0, 0));

        println!();
        println!("  Theme: {theme_name}");
        println!("  {:.0}% AA — {} scopes need attention", report.aa_percent(), failing.len());
        println!();

        let stdin = io::stdin();
        let mut reader = stdin.lock();

        // Track changes
        let mut palette_changes: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut scope_changes: Vec<(String, String, String)> = Vec::new();
        let mut acknowledged: Vec<String> = Vec::new();
        let mut fixed_palette_colors: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut quit = false;
        let mut done = false;

        // ── Phase 1: Palette opportunities ──
        // Group failing scopes by palette color, show those with 2+ scopes first
        let mut palette_failures: std::collections::HashMap<String, Vec<&str>> =
            std::collections::HashMap::new();
        for scope in &failing {
            if let Some(pal_name) = scope_to_palette.get(scope.name) {
                palette_failures
                    .entry(pal_name.clone())
                    .or_default()
                    .push(scope.name);
            }
        }

        let mut palette_opportunities: Vec<_> = palette_failures
            .iter()
            .filter(|(_, scopes)| scopes.len() >= 2)
            .collect();
        palette_opportunities.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        if !palette_opportunities.is_empty() {
            println!("  ─── Step 1: Palette changes (high impact) ───");
            println!();
            println!("  These palette colors affect multiple failing scopes.");
            println!("  Fixing them first reduces the number of individual scopes to review.");
            println!();

            for (i, (pal_name, scopes)) in palette_opportunities.iter().enumerate() {
                if quit || done {
                    break;
                }

                let pal_hex = match palette.get(*pal_name) {
                    Some(h) => h.clone(),
                    None => continue,
                };
                let pal_rgb = match parse_hex(&pal_hex) {
                    Some(rgb) => rgb,
                    None => continue,
                };

                let suggestions = suggest_fixes(pal_rgb, theme_bg_rgb);
                if suggestions.is_empty() {
                    continue;
                }

                println!(
                    "  [{}/{}] Palette '{}' = \"{}\" ({} failing scopes)",
                    i + 1,
                    palette_opportunities.len(),
                    pal_name,
                    pal_hex,
                    scopes.len(),
                );
                println!("    Affects: {}", scopes.join(", "));

                // Show current rendering with first scope as example
                let preview = ansi_colored_text(
                    &format!(" {} ", scopes[0]),
                    pal_rgb,
                    theme_bg_rgb,
                );
                let ratio = contrast_ratio(pal_rgb, theme_bg_rgb);
                println!(
                    "    Current: {} — {} ({:.1}:1)",
                    preview,
                    Readability::from_ratio(ratio).label(),
                    ratio,
                );

                for (j, s) in suggestions.iter().enumerate() {
                    let orig_lum = helix_view::accessibility::relative_luminance(
                        s.original.0, s.original.1, s.original.2,
                    );
                    let sugg_lum = helix_view::accessibility::relative_luminance(
                        s.suggested.0, s.suggested.1, s.suggested.2,
                    );
                    let direction = if sugg_lum > orig_lum { "Lighten" } else { "Darken" };
                    let what = match s.target {
                        AdjustTarget::Foreground => "color",
                        AdjustTarget::Background => "background",
                    };
                    let level_label = match s.target_level {
                        WcagLevel::AA => "readable",
                        WcagLevel::AAA => "easy to read",
                        WcagLevel::Fail => continue,
                    };
                    // Only show fg suggestions for palette (bg is scope-specific)
                    if s.target == AdjustTarget::Background {
                        continue;
                    }
                    let preview = ansi_colored_text(
                        &format!(" {} ", scopes[0]),
                        s.suggested,
                        theme_bg_rgb,
                    );
                    println!(
                        "    [{}] {} {} to {} — {} ({:.1}:1)",
                        j + 1,
                        direction,
                        what,
                        rgb_to_hex(s.suggested.0, s.suggested.1, s.suggested.2),
                        level_label,
                        s.suggested_ratio,
                    );
                    println!("        {}", preview);
                }
                println!("    [s] Skip  [c] Custom color  [d] Done (save & stop)  [q] Quit (discard)");

                print!("    > ");
                io::stdout().flush()?;
                let mut input = String::new();
                reader.read_line(&mut input)?;
                let input = input.trim().to_lowercase();

                match input.as_str() {
                    "s" => {
                        println!("    → Skipped");
                    }
                    "d" => {
                        println!("    → Saving changes so far");
                        done = true;
                    }
                    "q" => {
                        quit = true;
                    }
                    "c" => {
                        print!("    Enter hex color (e.g. #AABBCC): ");
                        io::stdout().flush()?;
                        let mut hex_input = String::new();
                        reader.read_line(&mut hex_input)?;
                        if let Some(custom_rgb) = parse_hex(hex_input.trim()) {
                            let hex = rgb_to_hex(custom_rgb.0, custom_rgb.1, custom_rgb.2);
                            palette_changes.insert((*pal_name).clone(), hex.clone());
                            fixed_palette_colors.insert((*pal_name).clone());
                            println!(
                                "    → Palette '{}' = \"{}\" (fixes {} scopes)",
                                pal_name, hex, scopes.len(),
                            );
                        } else {
                            println!("    → Invalid hex color, skipping");
                        }
                    }
                    n => {
                        // Only fg suggestions shown, filter to those
                        let fg_suggestions: Vec<_> = suggestions
                            .iter()
                            .filter(|s| s.target == AdjustTarget::Foreground)
                            .collect();
                        if let Ok(idx) = n.parse::<usize>() {
                            if idx >= 1 && idx <= fg_suggestions.len() {
                                let s = fg_suggestions[idx - 1];
                                let hex = rgb_to_hex(s.suggested.0, s.suggested.1, s.suggested.2);
                                palette_changes.insert((*pal_name).clone(), hex.clone());
                                fixed_palette_colors.insert((*pal_name).clone());
                                println!(
                                    "    → Palette '{}' = \"{}\" (fixes {} scopes)",
                                    pal_name, hex, scopes.len(),
                                );
                            } else {
                                println!("    → Invalid choice, skipping");
                            }
                        } else {
                            println!("    → Unknown input, skipping");
                        }
                    }
                }
                println!();
            }
        }

        // ── Phase 2: Remaining individual scopes ──
        let remaining: Vec<_> = failing
            .iter()
            .filter(|scope| {
                // Skip scopes already fixed by palette changes
                if let Some(pal_name) = scope_to_palette.get(scope.name) {
                    if fixed_palette_colors.contains(pal_name) {
                        return false;
                    }
                }
                true
            })
            .collect();

        if !remaining.is_empty() && !quit && !done {
            println!(
                "  ─── Step 2: Individual scopes ({} remaining) ───",
                remaining.len(),
            );
            println!();
            println!("    [1-N] Pick a suggestion");
            println!("    [s]   Skip — accept this contrast for stylistic reasons");
            println!("    [c]   Enter a custom hex color");
            println!("    [q]   Quit and show results so far");
            println!();

            for (i, scope) in remaining.iter().enumerate() {
                if quit || done {
                    break;
                }

                let (fg, bg) =
                    helix_view::accessibility::resolve_contrast_pair(&theme, scope.name);
                let fg_rgb = match fg.and_then(|c| resolve_color(c).rgb()) {
                    Some(rgb) => rgb,
                    None => continue,
                };
                let bg_rgb = match bg.and_then(|c| resolve_color(c).rgb()) {
                    Some(rgb) => rgb,
                    None => continue,
                };

                let ratio = contrast_ratio(fg_rgb, bg_rgb);
                let readability = Readability::from_ratio(ratio);

                println!("  [{}/{}] {}", i + 1, remaining.len(), scope.name);
                let preview = ansi_colored_text(
                    &format!(" {} ", scope.name),
                    fg_rgb,
                    bg_rgb,
                );
                println!(
                    "    Current: {} — {} ({:.1}:1)",
                    preview,
                    readability.label(),
                    ratio,
                );

                let suggestions = suggest_fixes(fg_rgb, bg_rgb);
                if suggestions.is_empty() {
                    println!("    No automatic suggestions available.");
                    println!();
                    continue;
                }

                for (j, s) in suggestions.iter().enumerate() {
                    let orig_lum = helix_view::accessibility::relative_luminance(
                        s.original.0, s.original.1, s.original.2,
                    );
                    let sugg_lum = helix_view::accessibility::relative_luminance(
                        s.suggested.0, s.suggested.1, s.suggested.2,
                    );
                    let direction = if sugg_lum > orig_lum { "Lighten" } else { "Darken" };
                    let what = match s.target {
                        AdjustTarget::Foreground => "text",
                        AdjustTarget::Background => "background",
                    };
                    let level_label = match s.target_level {
                        WcagLevel::AA => "readable",
                        WcagLevel::AAA => "easy to read",
                        WcagLevel::Fail => continue,
                    };
                    let (preview_fg, preview_bg) = match s.target {
                        AdjustTarget::Foreground => (s.suggested, bg_rgb),
                        AdjustTarget::Background => (fg_rgb, s.suggested),
                    };
                    let preview = ansi_colored_text(
                        &format!(" {} ", scope.name),
                        preview_fg,
                        preview_bg,
                    );
                    println!(
                        "    [{}] {} {} to {} — {} ({:.1}:1)",
                        j + 1,
                        direction,
                        what,
                        rgb_to_hex(s.suggested.0, s.suggested.1, s.suggested.2),
                        level_label,
                        s.suggested_ratio,
                    );
                    println!("        {}", preview);
                }
                println!("    [s] Skip  [c] Custom color  [d] Done (save & stop)  [q] Quit (discard)");

                print!("    > ");
                io::stdout().flush()?;
                let mut input = String::new();
                reader.read_line(&mut input)?;
                let input = input.trim().to_lowercase();

                match input.as_str() {
                    "s" => {
                        acknowledged.push(scope.name.to_string());
                        println!("    → Skipped (will add to acknowledged list)");
                    }
                    "d" => {
                        println!("    → Saving changes so far");
                        done = true;
                    }
                    "q" => {
                        quit = true;
                    }
                    "c" => {
                        print!("    Enter hex color (e.g. #AABBCC): ");
                        io::stdout().flush()?;
                        let mut hex_input = String::new();
                        reader.read_line(&mut hex_input)?;
                        if let Some(custom_rgb) = parse_hex(hex_input.trim()) {
                            let new_ratio = contrast_ratio(custom_rgb, bg_rgb);
                            let preview = ansi_colored_text(
                                &format!(" {} ", scope.name),
                                custom_rgb,
                                bg_rgb,
                            );
                            println!(
                                "    → {} — {} ({:.1}:1)",
                                preview,
                                Readability::from_ratio(new_ratio).label(),
                                new_ratio,
                            );
                            let hex = rgb_to_hex(custom_rgb.0, custom_rgb.1, custom_rgb.2);
                            scope_changes.push((
                                scope.name.to_string(),
                                "fg".to_string(),
                                hex,
                            ));
                        } else {
                            println!("    → Invalid hex color, skipping");
                            acknowledged.push(scope.name.to_string());
                        }
                    }
                    n => {
                        if let Ok(idx) = n.parse::<usize>() {
                            if idx >= 1 && idx <= suggestions.len() {
                                let s = &suggestions[idx - 1];
                                let hex =
                                    rgb_to_hex(s.suggested.0, s.suggested.1, s.suggested.2);
                                let what = match s.target {
                                    AdjustTarget::Foreground => "fg",
                                    AdjustTarget::Background => "bg",
                                };
                                scope_changes.push((
                                    scope.name.to_string(),
                                    what.to_string(),
                                    hex.clone(),
                                ));
                                println!("    → {} = \"{}\"", what, hex);
                            } else {
                                println!("    → Invalid choice, skipping");
                                acknowledged.push(scope.name.to_string());
                            }
                        } else {
                            println!("    → Unknown input, skipping");
                            acknowledged.push(scope.name.to_string());
                        }
                    }
                }
                println!();
            }
        }

        // ── Generate output theme file ──
        if quit {
            println!("  Quit — no changes saved.");
            return Ok(());
        }

        let has_changes =
            !palette_changes.is_empty() || !scope_changes.is_empty() || !acknowledged.is_empty();

        if !has_changes {
            println!("  No changes made.");
            return Ok(());
        }

        // Start from the merged TOML and apply changes
        let mut theme_table = merged_toml
            .and_then(|v| match v {
                toml::Value::Table(t) => Some(t),
                _ => None,
            })
            .unwrap_or_default();

        // Apply palette changes
        {
            let palette_table = theme_table
                .entry("palette".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
                .as_table_mut()
                .unwrap();
            for (name, hex) in &palette_changes {
                palette_table.insert(name.clone(), toml::Value::String(hex.clone()));
            }
        }

        // Apply scope changes
        for (scope, what, hex) in &scope_changes {
            let existing = theme_table.get(scope).cloned();
            let mut new_value = match &existing {
                Some(toml::Value::Table(t)) => t.clone(),
                Some(toml::Value::String(old_fg)) => {
                    let mut t = toml::map::Map::new();
                    if what == "bg" {
                        t.insert("fg".to_string(), toml::Value::String(old_fg.clone()));
                    }
                    t
                }
                _ => toml::map::Map::new(),
            };
            new_value.insert(what.clone(), toml::Value::String(hex.clone()));
            theme_table.insert(scope.clone(), toml::Value::Table(new_value));
        }

        // Detect repeated hex colors and auto-create palette entries
        let palette_lookup: std::collections::HashMap<String, String> = theme_table
            .get("palette")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| Some((v.as_str()?.to_uppercase(), k.clone())))
                    .collect()
            })
            .unwrap_or_default();

        let mut hex_usage: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (key, value) in &theme_table {
            if key == "palette" || key == "inherits" || key == "accessibility" || key == "rainbow"
            {
                continue;
            }
            let colors: Vec<String> = match value {
                toml::Value::String(s) => {
                    if s.starts_with('#') {
                        vec![s.to_uppercase()]
                    } else {
                        vec![]
                    }
                }
                toml::Value::Table(t) => {
                    let mut found = Vec::new();
                    for prop in ["fg", "bg"] {
                        if let Some(toml::Value::String(s)) = t.get(prop) {
                            if s.starts_with('#') {
                                found.push(s.to_uppercase());
                            }
                        }
                    }
                    found
                }
                _ => vec![],
            };
            for hex in colors {
                hex_usage.entry(hex).or_default().push(key.clone());
            }
        }

        // For any hex used 2+ times without a palette name, create one.
        // Try to derive the name from the original palette color it was adjusted from.
        let mut auto_palette: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut auto_counter = 0u32;

        // Build reverse map: new hex → original palette name (from palette_changes)
        let changed_hex_to_name: std::collections::HashMap<String, String> = palette_changes
            .iter()
            .map(|(name, hex)| (hex.to_uppercase(), name.clone()))
            .collect();

        let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (hex, scopes) in &hex_usage {
            if scopes.len() >= 2 && !palette_lookup.contains_key(hex) {
                let base_name = if let Some(original_name) = changed_hex_to_name.get(hex) {
                    format!("{}_a11y", original_name)
                } else {
                    let derived = scopes.iter().find_map(|scope_name| {
                        scope_to_palette.get(scope_name.as_str())
                    });
                    match derived {
                        Some(pal_name) => format!("{}_a11y", pal_name),
                        None => {
                            auto_counter += 1;
                            format!("color_{}", auto_counter)
                        }
                    }
                };

                // Ensure uniqueness — append a counter if the name was already used
                let name = if used_names.contains(&base_name) {
                    let mut n = 2;
                    loop {
                        let candidate = format!("{}_{}", base_name, n);
                        if !used_names.contains(&candidate) {
                            break candidate;
                        }
                        n += 1;
                    }
                } else {
                    base_name
                };

                used_names.insert(name.clone());
                auto_palette.insert(hex.clone(), name);
            }
        }

        // Add auto-palette entries to the palette table
        {
            let palette_table = theme_table
                .get_mut("palette")
                .and_then(|v| v.as_table_mut())
                .unwrap();
            for (hex, name) in &auto_palette {
                palette_table.insert(name.clone(), toml::Value::String(hex.clone()));
            }
        }

        // Replace raw hex references with palette names where possible
        let full_palette: std::collections::HashMap<String, String> = theme_table
            .get("palette")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| Some((v.as_str()?.to_uppercase(), k.clone())))
                    .collect()
            })
            .unwrap_or_default();

        let keys: Vec<String> = theme_table.keys().cloned().collect();
        for key in &keys {
            if key == "palette" || key == "inherits" || key == "accessibility" || key == "rainbow"
            {
                continue;
            }
            let value = theme_table.get(key).unwrap().clone();
            match value {
                toml::Value::String(ref s) if s.starts_with('#') => {
                    if let Some(pal_name) = full_palette.get(&s.to_uppercase()) {
                        theme_table
                            .insert(key.clone(), toml::Value::String(pal_name.clone()));
                    }
                }
                toml::Value::Table(ref t) => {
                    let mut new_t = t.clone();
                    for prop in ["fg", "bg"] {
                        if let Some(toml::Value::String(s)) = t.get(prop) {
                            if s.starts_with('#') {
                                if let Some(pal_name) = full_palette.get(&s.to_uppercase()) {
                                    new_t.insert(
                                        prop.to_string(),
                                        toml::Value::String(pal_name.clone()),
                                    );
                                }
                            }
                        }
                    }
                    theme_table.insert(key.clone(), toml::Value::Table(new_t));
                }
                _ => {}
            }
        }

        // Add acknowledged section
        if !acknowledged.is_empty() {
            let ack_array: Vec<toml::Value> = acknowledged
                .iter()
                .map(|s| toml::Value::String(s.clone()))
                .collect();
            let mut a11y_table = toml::map::Map::new();
            a11y_table.insert("acknowledged".to_string(), toml::Value::Array(ack_array));
            theme_table.insert("accessibility".to_string(), toml::Value::Table(a11y_table));
        }

        // Remove inherits — the output is a standalone theme with all values resolved
        theme_table.remove("inherits");

        // Write the file using flat key format (not TOML sections)
        let output_name = format!("{}-accessible", theme_name);
        let output_path = crate::path::themes().join(format!("{output_name}.toml"));

        let mut output = String::new();
        output.push_str(&format!("# {output_name}\n"));
        output.push_str(&format!("# Generated by: cargo xtask theme-fix {theme_name}\n"));
        output.push_str(&format!("# Based on: {theme_name}\n"));
        output.push_str("#\n");
        output.push_str(&format!(
            "# Accessibility improvements applied:\n#   Palette changes: {}\n#   Scope changes: {}\n#   Acknowledged: {}\n\n",
            palette_changes.len(), scope_changes.len(), acknowledged.len(),
        ));

        // Write scope entries as flat keys (sorted for consistency)
        let mut scope_keys: Vec<_> = theme_table
            .keys()
            .filter(|k| *k != "palette" && *k != "accessibility" && *k != "rainbow")
            .cloned()
            .collect();
        scope_keys.sort();

        for key in &scope_keys {
            let value = &theme_table[key];
            match value {
                toml::Value::String(s) => {
                    output.push_str(&format!("\"{}\" = \"{}\"\n", key, s));
                }
                toml::Value::Table(t) => {
                    let mut parts = Vec::new();
                    for prop in ["fg", "bg"] {
                        if let Some(toml::Value::String(v)) = t.get(prop) {
                            parts.push(format!("{} = \"{}\"", prop, v));
                        }
                    }
                    if let Some(toml::Value::Table(ul)) = t.get("underline") {
                        let mut ul_parts = Vec::new();
                        if let Some(toml::Value::String(c)) = ul.get("color") {
                            ul_parts.push(format!("color = \"{}\"", c));
                        }
                        if let Some(toml::Value::String(s)) = ul.get("style") {
                            ul_parts.push(format!("style = \"{}\"", s));
                        }
                        if !ul_parts.is_empty() {
                            parts.push(format!("underline = {{ {} }}", ul_parts.join(", ")));
                        }
                    }
                    if let Some(toml::Value::Array(mods)) = t.get("modifiers") {
                        let mod_strs: Vec<String> = mods
                            .iter()
                            .filter_map(|m| m.as_str())
                            .map(|m| format!("\"{}\"", m))
                            .collect();
                        if !mod_strs.is_empty() {
                            parts.push(format!("modifiers = [{}]", mod_strs.join(", ")));
                        }
                    }
                    if parts.is_empty() {
                        // Table with no recognized properties — serialize as-is
                        output.push_str(&format!(
                            "\"{}\" = {}\n",
                            key,
                            toml::to_string(value).unwrap_or_default().trim()
                        ));
                    } else {
                        output.push_str(&format!(
                            "\"{}\" = {{ {} }}\n",
                            key,
                            parts.join(", ")
                        ));
                    }
                }
                toml::Value::Array(_) => {
                    // Rainbow or similar array entries
                    let s = toml::to_string(value).unwrap_or_default();
                    output.push_str(&format!("\"{}\" = {}\n", key, s.trim()));
                }
                _ => {}
            }
        }

        // Write palette
        if let Some(toml::Value::Table(pal)) = theme_table.get("palette") {
            output.push_str("\n[palette]\n");
            let mut pal_keys: Vec<_> = pal.keys().cloned().collect();
            pal_keys.sort();
            for key in &pal_keys {
                if let Some(toml::Value::String(hex)) = pal.get(key) {
                    output.push_str(&format!("{} = \"{}\"\n", key, hex));
                }
            }
        }

        // Write accessibility section
        if let Some(toml::Value::Table(a11y)) = theme_table.get("accessibility") {
            output.push_str("\n[accessibility]\n");
            if let Some(toml::Value::Array(ack)) = a11y.get("acknowledged") {
                let items: Vec<String> = ack
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| format!("\"{}\"", s))
                    .collect();
                output.push_str(&format!("acknowledged = [{}]\n", items.join(", ")));
            }
        }

        std::fs::write(&output_path, &output)
            .map_err(|e| format!("Failed to write {}: {e}", output_path.display()))?;

        println!("  ═══════════════════════════════════════════════");
        println!("  Wrote: {}", output_path.display());
        println!("  ═══════════════════════════════════════════════");
        println!();
        println!("  Summary:");
        println!("    Palette changes: {}", palette_changes.len());
        println!("    Scope changes:   {}", scope_changes.len());
        println!("    Acknowledged:    {}", acknowledged.len());
        if !auto_palette.is_empty() {
            println!(
                "    Auto-created palette entries: {} (for repeated hex colors)",
                auto_palette.len()
            );
        }
        println!();
        println!("  To use: :theme {output_name}");
        println!();

        Ok(())
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
        theme-fix <theme>          Interactive guided fix for accessibility issues.
                                   Walk through each failing scope and choose fixes.
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
            "theme-fix" => tasks::theme_fix(args)?,
            invalid => return Err(format!("Invalid task name: {}", invalid).into()),
        },
    };
    Ok(())
}
