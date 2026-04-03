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
            invalid => return Err(format!("Invalid task name: {}", invalid).into()),
        },
    };
    Ok(())
}
