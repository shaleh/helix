use helix_core::syntax::config::PathMapping;
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    ToRemote,
    ToLocal,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::ToRemote => write!(f, "to-remote"),
            Direction::ToLocal => write!(f, "to-local"),
        }
    }
}

/// LSP spec field names that contain `file://` URIs.
/// This list must be kept in sync with the LSP specification.
const URI_FIELD_NAMES: &[&str] = &[
    "uri",
    "targetUri",
    "oldUri",
    "newUri",
    "scopeUri",
    "documentUri",
    "rootUri",
    "baseUri",
];

/// Rewrite a single `file://` URI string by replacing a path prefix.
/// Returns `Some(new_uri)` if the prefix matched, `None` otherwise.
pub fn remap_file_uri(uri: &str, from_prefix: &str, to_prefix: &str) -> Option<String> {
    let path = uri.strip_prefix("file://")?;
    let stripped = path.strip_prefix(from_prefix)?;
    if stripped.is_empty() || stripped.starts_with('/') {
        Some(format!("file://{to_prefix}{stripped}"))
    } else {
        None // partial match like "/home/user/project2" matching "/home/user/project"
    }
}

/// Replace a plain path prefix (for `rootPath` which is not a URI).
/// Returns `Some(new_path)` if the prefix matched, `None` otherwise.
pub fn replace_path_prefix(path: &str, from: &str, to: &str) -> Option<String> {
    let stripped = path.strip_prefix(from)?;
    if stripped.is_empty() || stripped.starts_with('/') {
        Some(format!("{to}{stripped}"))
    } else {
        None
    }
}

/// Walk a `serde_json::Value` tree and rewrite `file://` URIs in known URI fields.
pub fn remap_uris_in_value(value: &mut Value, mappings: &[PathMapping], direction: Direction) {
    if mappings.is_empty() {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if URI_FIELD_NAMES.contains(&key.as_str()) {
                    if let Value::String(uri) = val {
                        for mapping in mappings {
                            let Some(local) = &mapping.local else {
                                continue;
                            };
                            let (from, to) = match direction {
                                Direction::ToRemote => (local.as_str(), mapping.remote.as_str()),
                                Direction::ToLocal => (mapping.remote.as_str(), local.as_str()),
                            };
                            if let Some(new_uri) = remap_file_uri(uri, from, to) {
                                log::debug!("path remap ({direction}): {uri} -> {new_uri}");
                                *uri = new_uri;
                                break;
                            }
                        }
                    }
                }
                remap_uris_in_value(val, mappings, direction);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                remap_uris_in_value(item, mappings, direction);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- remap_file_uri ---

    #[test]
    fn test_remap_file_uri_basic() {
        let result = remap_file_uri(
            "file:///home/user/project/src/main.rs",
            "/home/user/project",
            "/workspace",
        );
        assert_eq!(result.unwrap(), "file:///workspace/src/main.rs");
    }

    #[test]
    fn test_remap_file_uri_no_match() {
        let result = remap_file_uri(
            "file:///other/path/file.rs",
            "/home/user/project",
            "/workspace",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_remap_file_uri_exact_root() {
        let result = remap_file_uri(
            "file:///home/user/project",
            "/home/user/project",
            "/workspace",
        );
        assert_eq!(result.unwrap(), "file:///workspace");
    }

    #[test]
    fn test_remap_file_uri_partial_prefix_no_match() {
        // "/home/user/project2" should NOT match prefix "/home/user/project"
        let result = remap_file_uri(
            "file:///home/user/project2/src/main.rs",
            "/home/user/project",
            "/workspace",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_remap_file_uri_not_file_scheme() {
        assert!(remap_file_uri("untitled:Untitled-1", "/home", "/workspace").is_none());
        assert!(remap_file_uri("https://example.com", "/home", "/workspace").is_none());
    }

    #[test]
    fn test_remap_file_uri_roundtrip() {
        let original = "file:///home/user/project/src/main.rs";
        let to_remote =
            remap_file_uri(original, "/home/user/project", "/workspace").unwrap();
        let back =
            remap_file_uri(&to_remote, "/workspace", "/home/user/project").unwrap();
        assert_eq!(back, original);
    }

    // --- replace_path_prefix ---

    #[test]
    fn test_replace_path_prefix_basic() {
        let result = replace_path_prefix(
            "/home/user/project/src/main.rs",
            "/home/user/project",
            "/workspace",
        );
        assert_eq!(result.unwrap(), "/workspace/src/main.rs");
    }

    #[test]
    fn test_replace_path_prefix_no_match() {
        let result =
            replace_path_prefix("/other/path/file.rs", "/home/user/project", "/workspace");
        assert!(result.is_none());
    }

    #[test]
    fn test_replace_path_prefix_partial_no_match() {
        let result = replace_path_prefix(
            "/home/user/project2/file.rs",
            "/home/user/project",
            "/workspace",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_replace_path_prefix_exact() {
        let result =
            replace_path_prefix("/home/user/project", "/home/user/project", "/workspace");
        assert_eq!(result.unwrap(), "/workspace");
    }

    // --- remap_uris_in_value ---

    #[test]
    fn test_remap_uris_in_value_nested() {
        let mappings = vec![PathMapping {
            local: Some("/home/user/project".into()),
            remote: "/workspace".into(),
        }];
        let mut value = json!({
            "uri": "file:///home/user/project/src/main.rs",
            "nested": {
                "targetUri": "file:///home/user/project/src/lib.rs"
            },
            "message": "Error in file:///home/user/project/foo"
        });
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(value["uri"], "file:///workspace/src/main.rs");
        assert_eq!(
            value["nested"]["targetUri"],
            "file:///workspace/src/lib.rs"
        );
        // Non-URI field must NOT be changed
        assert_eq!(
            value["message"],
            "Error in file:///home/user/project/foo"
        );
    }

    #[test]
    fn test_remap_uris_in_value_array() {
        let mappings = vec![PathMapping {
            local: Some("/home/user/project".into()),
            remote: "/workspace".into(),
        }];
        let mut value = json!([
            { "uri": "file:///home/user/project/a.rs" },
            { "uri": "file:///home/user/project/b.rs" },
        ]);
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(value[0]["uri"], "file:///workspace/a.rs");
        assert_eq!(value[1]["uri"], "file:///workspace/b.rs");
    }

    #[test]
    fn test_remap_uris_in_value_multiple_mappings() {
        let mappings = vec![
            PathMapping {
                local: Some("/home/user/project".into()),
                remote: "/workspace".into(),
            },
            PathMapping {
                local: Some("/home/user/.cargo".into()),
                remote: "/root/.cargo".into(),
            },
        ];
        let mut value = json!({
            "uri": "file:///home/user/.cargo/registry/src/foo.rs"
        });
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(
            value["uri"],
            "file:///root/.cargo/registry/src/foo.rs"
        );
    }

    #[test]
    fn test_remap_uris_in_value_to_local() {
        let mappings = vec![PathMapping {
            local: Some("/home/user/project".into()),
            remote: "/workspace".into(),
        }];
        let mut value = json!({
            "uri": "file:///workspace/src/main.rs"
        });
        remap_uris_in_value(&mut value, &mappings, Direction::ToLocal);
        assert_eq!(value["uri"], "file:///home/user/project/src/main.rs");
    }

    #[test]
    fn test_remap_uris_in_value_no_mappings_is_noop() {
        let mappings: Vec<PathMapping> = vec![];
        let original = json!({"uri": "file:///home/user/project/src/main.rs"});
        let mut value = original.clone();
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(value, original);
    }

    #[test]
    fn test_remap_skips_mapping_with_none_local() {
        let mappings = vec![PathMapping {
            local: None,
            remote: "/workspace".into(),
        }];
        let original = json!({"uri": "file:///home/user/project/src/main.rs"});
        let mut value = original.clone();
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(value, original);
    }

    #[test]
    fn test_remap_uris_in_value_non_file_uri_unchanged() {
        let mappings = vec![PathMapping {
            local: Some("/home/user/project".into()),
            remote: "/workspace".into(),
        }];
        let mut value = json!({
            "uri": "untitled:Untitled-1"
        });
        let original = value.clone();
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(value, original);
    }

    #[test]
    fn test_remap_all_uri_field_names() {
        let mappings = vec![PathMapping {
            local: Some("/local".into()),
            remote: "/remote".into(),
        }];
        let mut value = json!({
            "uri": "file:///local/a",
            "targetUri": "file:///local/b",
            "oldUri": "file:///local/c",
            "newUri": "file:///local/d",
            "scopeUri": "file:///local/e",
            "documentUri": "file:///local/f",
            "rootUri": "file:///local/g",
            "baseUri": "file:///local/h",
            "notAUri": "file:///local/z"
        });
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        assert_eq!(value["uri"], "file:///remote/a");
        assert_eq!(value["targetUri"], "file:///remote/b");
        assert_eq!(value["oldUri"], "file:///remote/c");
        assert_eq!(value["newUri"], "file:///remote/d");
        assert_eq!(value["scopeUri"], "file:///remote/e");
        assert_eq!(value["documentUri"], "file:///remote/f");
        assert_eq!(value["rootUri"], "file:///remote/g");
        assert_eq!(value["baseUri"], "file:///remote/h");
        // Unknown field name — must NOT be remapped
        assert_eq!(value["notAUri"], "file:///local/z");
    }

    #[test]
    fn test_remap_first_mapping_wins() {
        // Both mappings could match; first one should win
        let mappings = vec![
            PathMapping {
                local: Some("/home/user".into()),
                remote: "/container/user".into(),
            },
            PathMapping {
                local: Some("/home/user/project".into()),
                remote: "/workspace".into(),
            },
        ];
        let mut value = json!({
            "uri": "file:///home/user/project/src/main.rs"
        });
        remap_uris_in_value(&mut value, &mappings, Direction::ToRemote);
        // First mapping matches: /home/user -> /container/user
        assert_eq!(
            value["uri"],
            "file:///container/user/project/src/main.rs"
        );
    }
}
