//! Tests for the `list_files` action, in `src/actor/list_files.rs`.

use std::path::PathBuf;

use atama::actor::Actor;
use atama::actor::list_files::{check_pattern, glob_match};

/// Makes the fresh directory `<target tmp>/<name>` with the files `files`
/// and the subdirectories `dirs`, and returns its path.
fn make_dir(name: &str, files: &[&str], dirs: &[&str]) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for file in files {
        std::fs::write(dir.join(file), "x").unwrap();
    }
    for sub in dirs {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    dir.to_string_lossy().into_owned()
}

fn list(dir: Option<&str>, pattern: &str, input: &str) -> Result<String, String> {
    Actor::ListFiles {
        dir: dir.map(str::to_string),
        pattern: pattern.to_string(),
    }
    .act(input)
}

#[test]
fn glob_matches_stars_question_marks_and_classes() {
    let cases = [
        ("*", "a.txt", true),
        ("*", "", true),
        ("*.txt", "a.txt", true),
        ("*.txt", "a.txt.bak", false),
        ("a*b*c", "aXXbYYc", true),
        ("a*b*c", "aXXbYY", false),
        ("*a", "banana", true),
        ("?.md", "a.md", true),
        ("?.md", "ab.md", false),
        ("[abc].rs", "b.rs", true),
        ("[abc].rs", "d.rs", false),
        ("[a-c]*", "cat", true),
        ("[a-c]*", "dog", false),
        ("[!a-c]*", "dog", true),
        ("[^a-c]*", "cat", false),
        ("[]a]", "]", true),
        ("[a-]", "-", true),
        ("\\*", "*", true),
        ("\\*", "a", false),
        ("a[", "a[", true),
        ("a[", "ab", false),
        ("report", "report", true),
        ("report", "reports", false),
        ("*", ".env", false),
        (".*", ".env", true),
        ("?env", ".env", false),
        ("日*", "日本.txt", true),
    ];
    for (pattern, name, expected) in cases {
        assert_eq!(glob_match(pattern, name), expected, "{pattern} on {name}");
    }
}

#[test]
fn check_pattern_rejects_bad_patterns() {
    for pattern in ["*", "*.md", "[]a]", "\\[", "[!x]y"] {
        assert_eq!(check_pattern(pattern), Ok(()), "{pattern}");
    }
    assert_eq!(
        check_pattern(""),
        Err("the `list_files` pattern is empty".to_string())
    );
    assert_eq!(
        check_pattern("src/*.rs"),
        Err("the `list_files` pattern `src/*.rs` has a `/`, but `list_files` does not \
             go into subdirectories"
            .to_string())
    );
    assert_eq!(
        check_pattern("*.[rs"),
        Err("the `list_files` pattern `*.[rs` has a `[` without a closing `]`".to_string())
    );
}

#[test]
fn lists_the_matching_files_sorted_without_subdirectories() {
    let dir = make_dir(
        "list_files_sorted",
        &["b.md", "a.md", "c.txt", ".hidden.md"],
        &["sub.md", "nested"],
    );
    std::fs::write(PathBuf::from(&dir).join("nested").join("deep.md"), "x").unwrap();

    assert_eq!(
        list(Some(&dir), "*", "ignored input"),
        Ok(format!("{dir}/a.md\n{dir}/b.md\n{dir}/c.txt\n"))
    );
    assert_eq!(
        list(Some(&dir), "*.md", ""),
        Ok(format!("{dir}/a.md\n{dir}/b.md\n"))
    );
    assert_eq!(
        list(Some(&dir), ".*", ""),
        Ok(format!("{dir}/.hidden.md\n"))
    );
    assert_eq!(list(Some(&dir), "*.rs", ""), Ok(String::new()));
}

#[cfg(unix)]
#[test]
fn lists_symbolic_links_to_files() {
    let dir = make_dir("list_files_links", &["real.txt"], &["folder"]);
    let path = PathBuf::from(&dir);
    std::os::unix::fs::symlink(path.join("real.txt"), path.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(path.join("folder"), path.join("link-dir")).unwrap();
    assert_eq!(
        list(Some(&dir), "*", ""),
        Ok(format!("{dir}/link.txt\n{dir}/real.txt\n"))
    );
}

#[test]
fn reads_the_directory_from_the_input() {
    let dir = make_dir("list_files_input", &["one.txt"], &[]);
    assert_eq!(
        list(None, "*", &format!("  {dir}\n")),
        Ok(format!("{dir}/one.txt\n"))
    );

    let other = make_dir("list_files_input_other", &["two.txt"], &[]);
    let actor: Actor = "foreach list_files pattern=*.txt".parse().unwrap();
    assert_eq!(
        actor.act(&format!("{dir}\n{other}\n")),
        Ok(format!("{dir}/one.txt\n{other}/two.txt\n"))
    );
}

#[test]
fn fails_without_a_single_readable_directory() {
    assert_eq!(
        list(None, "*", " \n"),
        Err("`list_files` needs a directory in `dir=` or in its input".to_string())
    );
    assert_eq!(
        list(None, "*", "a\nb\n"),
        Err("`list_files` lists one directory, but its input has 2 lines. To list one \
             directory for each line, use `foreach list_files`"
            .to_string())
    );
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("list_files_no_such_dir");
    let missing = missing.to_string_lossy();
    let err = list(Some(&missing), "*", "").unwrap_err();
    assert!(
        err.starts_with(&format!("cannot list directory `{missing}`: ")),
        "{err}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn fails_on_a_file_name_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = make_dir("list_files_not_utf8", &[], &[]);
    std::fs::write(PathBuf::from(&dir).join(OsStr::from_bytes(b"bad\xff")), "x").unwrap();
    let err = list(Some(&dir), "*", "").unwrap_err();
    assert!(err.contains("is not UTF-8"), "{err}");
}
