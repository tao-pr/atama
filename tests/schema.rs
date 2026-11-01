//! Tests for the output schemas of agent actions.

use std::fs;
use std::path::PathBuf;

use serde_json::json;

use atama::schema::{OutputSchema, Schemas, check_schema, is_name, load, resolve};

/// A schema for an object with a string `title` and a number `count`.
fn article() -> OutputSchema {
    OutputSchema {
        name: "article".into(),
        schema: json!({
            "type": "object",
            "properties": {"title": {"type": "string"}, "count": {"type": "integer"}},
            "required": ["title"],
        }),
    }
}

/// Writes `text` to a new file `<name>` in a test directory of this
/// process, and returns its path.
fn temp_file(name: &str, text: &str) -> String {
    let dir: PathBuf = std::env::temp_dir().join(format!("atama-schema-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_name_has_only_letters_digits_underscores_and_hyphens() {
    for name in ["article", "a_b-1", &"x".repeat(64)] {
        assert!(is_name(name), "{name}");
    }
    for value in ["", "a.json", "dir/a", "./a", "a b", "é", &"x".repeat(65)] {
        assert!(!is_name(value), "{value}");
    }
}

#[test]
fn check_schema_accepts_only_valid_schema_objects() {
    assert_eq!(check_schema(&json!({}), "s"), Ok(()));
    assert_eq!(check_schema(&article().schema, "s"), Ok(()));
    assert_eq!(
        check_schema(&json!(true), "s"),
        Err("s is not a JSON object, as in `{\"type\": \"object\"}`".to_string())
    );
    let err = check_schema(&json!({"type": 5}), "s").unwrap_err();
    assert!(
        err.starts_with("s is not a valid JSON schema at `/type`: "),
        "{err}"
    );
    // An error at the root of the schema has no path.
    let err = check_schema(&json!({"$ref": "https://example.com/s.json"}), "s").unwrap_err();
    assert!(err.starts_with("s is not a valid JSON schema: "), "{err}");
}

#[test]
fn resolve_finds_the_named_block_and_skips_paths() {
    let mut schemas = Schemas::new();
    schemas.insert("article".into(), article());
    let params = |value: &str| vec!["model=m".to_string(), format!("output_schema={value}")];
    assert_eq!(resolve(&params("article"), &schemas), Ok(Some(article())));
    assert_eq!(resolve(&params("a.json"), &schemas), Ok(None));
    assert_eq!(resolve(&["model=m".to_string()], &schemas), Ok(None));
    assert!(resolve(&params("story"), &schemas).is_err());
}

#[test]
fn load_uses_the_block_for_a_name() {
    assert_eq!(load("article", Some(&article())), Ok(article()));
    for block in [None, Some(&article())] {
        assert_eq!(
            load("story", block),
            Err("output schema `story` is not defined in the job".to_string())
        );
    }
}

#[test]
fn load_reads_a_schema_file_for_a_path() {
    let path = temp_file("my article.v2.json", &article().schema.to_string());
    assert_eq!(
        load(&path, None),
        Ok(OutputSchema {
            name: "my_article_v2".into(),
            schema: article().schema,
        })
    );
    // Each character that a name cannot have becomes `_`.
    let path = temp_file(".json", "{}");
    assert_eq!(load(&path, None).unwrap().name, "_json");
    let path = temp_file("é.json", "{}");
    assert_eq!(load(&path, None).unwrap().name, "_");
    let dir = std::env::temp_dir().join(format!("atama-schema-{}", std::process::id()));
    let path = format!("{}/", dir.to_string_lossy());
    let err = load(&path, None).unwrap_err();
    assert!(err.starts_with("cannot read output schema file"), "{err}");
}

#[test]
fn load_fails_on_a_bad_schema_file() {
    let err = load("./no/such/schema.json", None).unwrap_err();
    assert!(
        err.starts_with("cannot read output schema file `./no/such/schema.json`: "),
        "{err}"
    );

    let path = temp_file("bad.json", "{");
    let err = load(&path, None).unwrap_err();
    assert!(
        err.starts_with(&format!("output schema file `{path}` is not valid JSON: ")),
        "{err}"
    );

    let path = temp_file("list.json", "[]");
    assert_eq!(
        load(&path, None),
        Err(format!(
            "output schema file `{path}` is not a JSON object, as in `{{\"type\": \"object\"}}`"
        ))
    );
}

#[test]
fn response_format_has_the_name_and_the_schema() {
    assert_eq!(
        article().response_format(),
        json!({
            "type": "json_schema",
            "json_schema": {"name": "article", "schema": article().schema},
        })
    );
}

#[test]
fn instruction_has_the_schema() {
    let instruction = article().instruction();
    assert!(
        instruction
            .starts_with("Your final reply, without tool calls, must be only one JSON value"),
        "{instruction}"
    );
    assert!(
        instruction.ends_with(&serde_json::to_string_pretty(&article().schema).unwrap()),
        "{instruction}"
    );
}

#[test]
fn check_reply_returns_the_json_of_a_valid_reply() {
    let schema = article();
    assert_eq!(
        schema.check_reply(" {\"title\": \"T\"}\n"),
        Ok("{\"title\": \"T\"}".to_string())
    );
    // A code fence is removed, with or without a language.
    // A fence without its closing fence is removed too.
    for reply in [
        "```json\n{\"title\": \"T\"}\n```",
        "```\n{\"title\": \"T\"}\n```\n",
        "```json\n{\"title\": \"T\"}",
    ] {
        assert_eq!(
            schema.check_reply(reply),
            Ok("{\"title\": \"T\"}".to_string()),
            "{reply}"
        );
    }
}

#[test]
fn check_reply_fails_on_text_that_is_not_json() {
    for reply in ["", "The title is T.", "```json"] {
        let err = article().check_reply(reply).unwrap_err();
        assert!(
            err.starts_with("the reply is not valid JSON: "),
            "{reply}: {err}"
        );
    }
}

#[test]
fn check_reply_lists_the_validation_errors() {
    let schema = article();
    let err = schema.check_reply("{\"count\": \"two\"}").unwrap_err();
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines[0], "the reply does not match the schema:");
    assert_eq!(lines.len(), 3, "{err}");
    assert!(err.contains("\nat `/`: "), "{err}");
    assert!(err.contains("\nat `/count`: "), "{err}");

    // At most five errors.
    let many = OutputSchema {
        name: "many".into(),
        schema: json!({"type": "array", "items": {"type": "string"}}),
    };
    let err = many.check_reply("[1, 2, 3, 4, 5, 6, 7]").unwrap_err();
    assert_eq!(err.lines().count(), 6, "{err}");
}
