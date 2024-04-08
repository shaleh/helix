/// Increment a boolean.
///
pub fn increment(selected_text: &str, _: i64) -> Option<String> {
    if selected_text.is_empty() {
        return None;
    }

    let incremented = match selected_text {
        "false" => "true",
        "true" => "false",
        "False" => "True",
        "True" => "False",
        "FALSE" => "TRUE",
        "TRUE" => "FALSE",
        _ => return None,
    };

    Some(String::from(incremented))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_boolean_increment() {
        let tests = [
            ("false", "true"),
            ("true", "false"),
            ("False", "True"),
            ("True", "False"),
            ("FALSE", "TRUE"),
            ("TRUE", "FALSE"),
        ];

        for (original, expected) in tests {
            assert_eq!(increment(original, 0), Some(String::from(expected)));
        }
    }
}
