use crate::keymap;
use crate::keymap::{merge_keys, KeyTrie};
use helix_loader::merge_toml_values;
use helix_view::editor::AliasEntry;
use helix_view::{document::Mode, theme};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::fs;
use std::io::Error as IOError;
use std::sync::Arc;
use toml::de::Error as TomlError;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub theme: Option<theme::Config>,
    pub keys: HashMap<Mode, KeyTrie>,
    pub editor: helix_view::editor::Config,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRaw {
    pub theme: Option<theme::Config>,
    pub keys: Option<HashMap<Mode, KeyTrie>>,
    pub editor: Option<toml::Value>,
    pub commands: Option<HashMap<String, toml::Value>>,
}

/// Parse the raw `[commands]` table into typed [`AliasEntry`] values, applying
/// the leading-`:` visibility convention and rejecting unsupported forms with
/// errors that name the offending alias key.
fn parse_alias_entries(
    raw: HashMap<String, toml::Value>,
) -> Result<HashMap<String, Arc<AliasEntry>>, ConfigLoadError> {
    use serde::de::Error as _;
    let mut out = HashMap::with_capacity(raw.len());
    for (key, value) in raw {
        let visible = key.starts_with(':');
        let bare = key.trim_start_matches(':').to_string();

        let commands = match value {
            toml::Value::String(s) => vec![strip_leading_colon(s)],
            toml::Value::Array(arr) => {
                let mut cmds = Vec::with_capacity(arr.len());
                for item in arr {
                    match item {
                        toml::Value::String(s) => cmds.push(strip_leading_colon(s)),
                        _ => {
                            return Err(ConfigLoadError::BadConfig(TomlError::custom(format!(
                                "commands.{key}: array entries must be command strings"
                            ))));
                        }
                    }
                }
                cmds
            }
            toml::Value::Table(_) => {
                return Err(ConfigLoadError::BadConfig(TomlError::custom(format!(
                    "commands.{key}: inline-table form (desc/accepts/completer) is not yet supported in this fork; use a string or array of strings"
                ))));
            }
            _ => {
                return Err(ConfigLoadError::BadConfig(TomlError::custom(format!(
                    "commands.{key}: value must be a string or array of strings"
                ))));
            }
        };

        for cmd in &commands {
            if cmd.contains("%arg{") {
                return Err(ConfigLoadError::BadConfig(TomlError::custom(format!(
                    "commands.{key}: positional argument placeholders ('%arg{{...}}') are not yet supported in this fork"
                ))));
            }
        }

        out.insert(bare, Arc::new(AliasEntry { commands, visible }));
    }
    Ok(out)
}

fn strip_leading_colon(s: String) -> String {
    if let Some(rest) = s.strip_prefix(':') {
        rest.to_string()
    } else {
        s
    }
}

impl Default for Config {
    fn default() -> Config {
        Config {
            theme: None,
            keys: keymap::default(),
            editor: helix_view::editor::Config::default(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigLoadError {
    BadConfig(TomlError),
    Error(IOError),
}

impl Default for ConfigLoadError {
    fn default() -> Self {
        ConfigLoadError::Error(IOError::new(std::io::ErrorKind::NotFound, "place holder"))
    }
}

impl Display for ConfigLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigLoadError::BadConfig(err) => err.fmt(f),
            ConfigLoadError::Error(err) => err.fmt(f),
        }
    }
}

impl Config {
    pub fn load(
        global: Result<&String, ConfigLoadError>,
        local: Result<String, ConfigLoadError>,
    ) -> Result<Config, ConfigLoadError> {
        let global_config: Result<ConfigRaw, ConfigLoadError> =
            global.and_then(|file| toml::from_str(file).map_err(ConfigLoadError::BadConfig));
        let local_config: Result<ConfigRaw, ConfigLoadError> =
            local.and_then(|file| toml::from_str(&file).map_err(ConfigLoadError::BadConfig));
        let res = match (global_config, local_config) {
            (Ok(global), Ok(local)) => {
                let mut keys = keymap::default();
                if let Some(global_keys) = global.keys {
                    merge_keys(&mut keys, global_keys)
                }
                if let Some(local_keys) = local.keys {
                    merge_keys(&mut keys, local_keys)
                }

                let mut editor = match (global.editor, local.editor) {
                    (None, None) => helix_view::editor::Config::default(),
                    (None, Some(val)) | (Some(val), None) => {
                        val.try_into().map_err(ConfigLoadError::BadConfig)?
                    }
                    (Some(global), Some(local)) => merge_toml_values(global, local, 3)
                        .try_into()
                        .map_err(ConfigLoadError::BadConfig)?,
                };

                let mut aliases = match global.commands {
                    Some(raw) => parse_alias_entries(raw)?,
                    None => HashMap::new(),
                };
                if let Some(raw) = local.commands {
                    // Workspace-local entries override global on key collision.
                    aliases.extend(parse_alias_entries(raw)?);
                }
                editor.commands = aliases;

                Config {
                    theme: local.theme.or(global.theme),
                    keys,
                    editor,
                }
            }
            // if any configs are invalid return that first
            (_, Err(ConfigLoadError::BadConfig(err)))
            | (Err(ConfigLoadError::BadConfig(err)), _) => {
                return Err(ConfigLoadError::BadConfig(err))
            }
            (Ok(config), Err(_)) | (Err(_), Ok(config)) => {
                let mut keys = keymap::default();
                if let Some(keymap) = config.keys {
                    merge_keys(&mut keys, keymap);
                }
                let mut editor = config.editor.map_or_else(
                    || Ok(helix_view::editor::Config::default()),
                    |val| val.try_into().map_err(ConfigLoadError::BadConfig),
                )?;
                if let Some(raw) = config.commands {
                    editor.commands = parse_alias_entries(raw)?;
                }
                Config {
                    theme: config.theme,
                    keys,
                    editor,
                }
            }

            // these are just two io errors return the one for the global config
            (Err(err), Err(_)) => return Err(err),
        };

        Ok(res)
    }

    pub fn load_default() -> Result<Config, ConfigLoadError> {
        let global_config =
            fs::read_to_string(helix_loader::config_file()).map_err(ConfigLoadError::Error)?;
        let local_config = fs::read_to_string(helix_loader::workspace_config_file())
            .map_err(ConfigLoadError::Error);

        let phony_config = ConfigLoadError::Error(IOError::other("hacky placeholder"));
        let global_parsed = Config::load(Ok(&global_config), Err(phony_config))?;
        if let helix_loader::workspace_trust::TrustStatus::Trusted =
            helix_loader::workspace_trust::quick_query_workspace(global_parsed.editor.insecure)
        {
            Config::load(Ok(&global_config), local_config)
        } else {
            Ok(global_parsed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Config {
        fn load_test(config: &str) -> Config {
            Config::load(Ok(&config.to_owned()), Err(ConfigLoadError::default())).unwrap()
        }
    }

    #[test]
    fn parsing_keymaps_config_file() {
        use crate::keymap;
        use helix_core::hashmap;
        use helix_view::document::Mode;

        let sample_keymaps = r#"
            [keys.insert]
            y = "move_line_down"
            S-C-a = "delete_selection"

            [keys.normal]
            A-F12 = "move_next_word_end"
        "#;

        let mut keys = keymap::default();
        merge_keys(
            &mut keys,
            hashmap! {
                Mode::Insert => keymap!({ "Insert mode"
                    "y" => move_line_down,
                    "S-C-a" => delete_selection,
                }),
                Mode::Normal => keymap!({ "Normal mode"
                    "A-F12" => move_next_word_end,
                }),
            },
        );

        assert_eq!(
            Config::load_test(sample_keymaps),
            Config {
                keys,
                ..Default::default()
            }
        );
    }

    #[test]
    fn keys_resolve_to_correct_defaults() {
        // From serde default
        let default_keys = Config::load_test("").keys;
        assert_eq!(default_keys, keymap::default());

        // From the Default trait
        let default_keys = Config::default().keys;
        assert_eq!(default_keys, keymap::default());
    }

    fn load_aliases(
        toml_str: &str,
    ) -> Result<HashMap<String, Arc<AliasEntry>>, ConfigLoadError> {
        Config::load(Ok(&toml_str.to_string()), Err(ConfigLoadError::default()))
            .map(|c| c.editor.commands)
    }

    #[test]
    fn aliases_string_form() {
        let aliases = load_aliases(
            r#"
            [commands]
            ":show-blame" = ":noop"
            "#,
        )
        .unwrap();
        let entry = aliases.get("show-blame").expect("alias should be present");
        assert_eq!(entry.commands, vec!["noop".to_string()]);
        assert!(entry.visible);
    }

    #[test]
    fn aliases_array_form_runs_in_order() {
        let aliases = load_aliases(
            r#"
            [commands]
            ":wq" = [":write", ":quit"]
            "#,
        )
        .unwrap();
        let entry = aliases.get("wq").unwrap();
        assert_eq!(entry.commands, vec!["write".to_string(), "quit".to_string()]);
        assert!(entry.visible);
    }

    #[test]
    fn aliases_unprefixed_key_is_hidden() {
        let aliases = load_aliases(
            r#"
            [commands]
            "scratch" = ":new"
            "#,
        )
        .unwrap();
        let entry = aliases.get("scratch").unwrap();
        assert!(!entry.visible);
    }

    #[test]
    fn aliases_strip_leading_colon_from_values() {
        let aliases = load_aliases(
            r#"
            [commands]
            ":a" = ":quit"
            ":b" = "quit"
            "#,
        )
        .unwrap();
        assert_eq!(aliases.get("a").unwrap().commands, vec!["quit".to_string()]);
        assert_eq!(aliases.get("b").unwrap().commands, vec!["quit".to_string()]);
    }

    #[test]
    fn aliases_inline_table_form_is_rejected() {
        let err = load_aliases(
            r#"
            [commands.":wcd"]
            commands = [":write"]
            desc = "x"
            "#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("wcd"), "error should name the alias: {msg}");
        assert!(
            msg.contains("inline-table"),
            "error should mention inline-table: {msg}"
        );
    }

    #[test]
    fn aliases_arg_placeholder_is_rejected() {
        let err = load_aliases(
            r#"
            [commands]
            ":greet" = ":echo hello %arg{0}"
            "#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("greet"), "error should name the alias: {msg}");
        assert!(msg.contains("%arg"), "error should cite %arg: {msg}");
    }

    #[test]
    fn aliases_survive_set_option_json_roundtrip() {
        // `set_option` and `toggle_option` rebuild `editor::Config` by
        // round-tripping it through `serde_json`. Verify that the `commands`
        // field survives that round-trip on its own, independent of the
        // explicit copy those call sites also perform.
        use helix_view::editor::{AliasEntry, Config as EditorConfig};

        let mut config = EditorConfig::default();
        config.commands.insert(
            "show-blame".to_string(),
            Arc::new(AliasEntry {
                commands: vec!["echo blame".to_string()],
                visible: true,
            }),
        );

        let json = serde_json::to_value(&config).unwrap();
        let restored: EditorConfig = serde_json::from_value(json).unwrap();
        let entry = restored
            .commands
            .get("show-blame")
            .expect("alias should survive JSON round-trip");
        assert_eq!(entry.commands, vec!["echo blame".to_string()]);
        assert!(entry.visible);
    }

    #[test]
    fn aliases_non_string_array_entry_is_rejected() {
        let err = load_aliases(
            r#"
            [commands]
            ":wq" = [":write", 42]
            "#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("wq"),
            "error should name the offending alias: {msg}"
        );
    }
}
