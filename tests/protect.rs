//! Tests for `protect=`: the patterns, the check of `write_file`, the
//! sandbox profile of `bash`, and the protected paths in the agent loop.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use atama::agent_loop_openai_compat::Tool;
use atama::agent_loop_openai_compat::tools::{Bash, WriteFile};
use atama::protect::{Protect, check_pattern, parse_patterns};

/// A new, empty directory for one test, as a real path.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atama-protect-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn protect(patterns: &[&str], cwd: &Path) -> Protect {
    let patterns: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();
    Protect::new(&patterns, cwd).unwrap()
}

fn path(dir: &Path, rest: &str) -> String {
    format!("{}/{rest}", dir.display())
}

#[test]
fn parses_patterns_separated_by_commas_or_lines() {
    assert_eq!(
        parse_patterns("src/**, Cargo.toml").unwrap(),
        vec!["src/**", "Cargo.toml"]
    );
    assert_eq!(
        parse_patterns("- src/**\n- Cargo.toml\n").unwrap(),
        vec!["src/**", "Cargo.toml"]
    );
    assert_eq!(
        parse_patterns(" , \n").unwrap_err(),
        "`protect=` has no pattern, as in `protect=\"src/**,Cargo.toml\"`"
    );
    assert_eq!(
        parse_patterns("src,a\\b").unwrap_err(),
        "the `protect` pattern `a\\b` has a `\"` or a `\\`"
    );
}

#[test]
fn rejects_invalid_patterns() {
    let cases = [
        ("a\"b", "has a `\"` or a `\\`"),
        (
            "src/a**",
            "has a `**` that is not a whole part of the path, as in `src/**/a.rs`",
        ),
        ("src/[ab", "has a `[` without a `]`"),
        ("src/[a/b]", "has a `[` without a `]`"),
        ("src/[]", "has an empty class `[]`"),
        ("src/[!]", "has an empty class `[]`"),
        (
            "src/[a+]",
            "has a class with another character than letters, digits, `-`, `_` and `.`",
        ),
        ("src/*/../a", "has a `..` after a part with a glob"),
    ];
    for (pattern, reason) in cases {
        assert_eq!(
            check_pattern(pattern),
            Err(format!("the `protect` pattern `{pattern}` {reason}")),
        );
    }
    let err = check_pattern("src/[z-a]").unwrap_err();
    assert!(
        err.starts_with("the `protect` pattern `src/[z-a]` is invalid: "),
        "{err}"
    );
    for pattern in [
        "src/**/[a-z_.]?.rs",
        "../a",
        "/etc/*",
        "[!x]*",
        "a.(b)+{c}|$^",
    ] {
        assert_eq!(check_pattern(pattern), Ok(()), "{pattern}");
    }
}

#[test]
fn check_write_blocks_the_paths_that_match_and_the_paths_below_them() {
    let dir = temp_dir("check");
    let protect = protect(
        &[
            &path(&dir, "keep"),
            &path(&dir, "**/*.lock"),
            &path(&dir, "data/[!x]?.txt"),
            &path(&dir, "./x/../data/*.(1)"),
        ],
        &dir,
    );
    for blocked in [
        "keep",
        "keep/sub/file",
        "c.lock",
        "a/b/c.lock",
        ".hidden.lock",
        "data/ab.txt",
        "data/v.(1)",
    ] {
        let target = path(&dir, blocked);
        let err = protect.check_write(&target).unwrap_err();
        assert!(
            err.starts_with(&format!("`{target}` is protected by `protect=")),
            "{err}"
        );
        assert!(err.ends_with("`, so the agent cannot write it"), "{err}");
    }
    for allowed in [
        "keeper",
        "data/xb.txt",
        "data/abc.txt",
        "a/c.lockx",
        "data/v.11",
    ] {
        assert_eq!(
            protect.check_write(&path(&dir, allowed)),
            Ok(()),
            "{allowed}"
        );
    }
    let err = protect.check_write(&path(&dir, "keep")).unwrap_err();
    assert_eq!(
        err,
        format!(
            "`{}` is protected by `protect={}`, so the agent cannot write it",
            path(&dir, "keep"),
            path(&dir, "keep")
        )
    );
}

#[test]
fn check_write_follows_symbolic_links_and_dot_dot() {
    let dir = temp_dir("links");
    fs::create_dir_all(dir.join("keep")).unwrap();
    fs::create_dir_all(dir.join("other")).unwrap();
    std::os::unix::fs::symlink(dir.join("keep"), dir.join("link")).unwrap();
    // A relative pattern is relative to the working directory of the run.
    let protect = protect(&["keep"], &dir);
    assert_eq!(protect.patterns(), vec!["keep"]);
    assert!(protect.check_write(&path(&dir, "link/file")).is_err());
    assert!(
        protect
            .check_write(&path(&dir, "other/../keep/file"))
            .is_err()
    );
    assert_eq!(protect.check_write(&path(&dir, "other/file")), Ok(()));
    assert_eq!(
        protect.check_write(&path(&dir, "missing/../keep")),
        Err(format!(
            "cannot resolve `{}`, because a `..` follows a directory that does not exist",
            path(&dir, "missing/../keep")
        ))
    );
    // A relative path is relative to the working directory of the process.
    assert_eq!(protect.check_write("target/atama-protect-free.txt"), Ok(()));
}

#[test]
fn the_working_directory_of_a_pattern_is_plain_text() {
    let dir = temp_dir("plain[a-z]*.d");
    let protect = protect(&["f", "../*/g"], &dir);
    assert!(protect.check_write(&path(&dir, "f")).is_err());
    assert_eq!(protect.check_write(&path(&dir, "h")), Ok(()));
    // In an absolute pattern, `[a-z]` and `*` are globs.
    let sibling = dir.with_file_name("atama-protect-plainx-sibling");
    fs::create_dir_all(&sibling).unwrap();
    assert!(protect.check_write(&path(&sibling, "g")).is_err());
}

#[test]
fn a_real_path_with_a_quote_is_an_error() {
    let dir = temp_dir("quote");
    fs::create_dir_all(dir.join("q\"d")).unwrap();
    std::os::unix::fs::symlink(dir.join("q\"d"), dir.join("qlink")).unwrap();
    let pattern = path(&dir, "qlink/x");
    let err = Protect::new(std::slice::from_ref(&pattern), &dir).unwrap_err();
    assert_eq!(
        err,
        format!(
            "the real path `{}` of the `protect` pattern `{pattern}` is not UTF-8 or has a `\"`",
            path(&dir, "q\"d/x")
        )
    );
    assert!(Protect::new(&["a**".to_string()], &dir).is_err());
}

#[test]
fn makes_the_sandbox_profile_and_the_instruction() {
    let protect = protect(
        &["/atama-missing/a/**/b", "/atama-missing/c"],
        Path::new("/"),
    );
    assert_eq!(
        protect.sandbox_profile(),
        "(version 1)\n(allow default)\n\
         (deny file-write* (regex #\"^/atama-missing/a(/.*)?/b(/.*)?$\" \
         #\"^/atama-missing/c(/.*)?$\"))\n\
         (deny file-write* (require-all (vnode-type DIRECTORY) (regex #\"^/$\" \
         #\"^/atama-missing$\" #\"^/atama-missing/a$\" #\"^/atama-missing/a(/.*)?$\")))\n"
    );
    assert_eq!(
        protect.instruction(),
        "Protected paths. You cannot write, delete or rename these paths, the paths below \
         them, or the directories above them. A tool call that tries fails: \
         `/atama-missing/a/**/b`, `/atama-missing/c`"
    );
}

#[test]
fn write_file_does_not_write_a_protected_path() {
    let dir = temp_dir("write");
    let mut tool = WriteFile::default();
    tool.set_protect(Some(Arc::new(protect(&["keep/**"], &dir))));
    let target = path(&dir, "keep/a.txt");
    let err = tool
        .call(&json!({"path": target, "content": "x"}))
        .unwrap_err();
    assert_eq!(
        err,
        format!("`{target}` is protected by `protect=keep/**`, so the agent cannot write it")
    );
    assert!(!dir.join("keep").exists());
    let free = path(&dir, "free.txt");
    tool.call(&json!({"path": free, "content": "x"})).unwrap();
    assert_eq!(fs::read_to_string(&free).unwrap(), "x");
}

/// Runs `command` with the `bash` tool and the protected paths `patterns`
/// of `dir`. Returns true if the command succeeds.
#[cfg(target_os = "macos")]
fn bash_succeeds(dir: &Path, patterns: &[&str], command: &str) -> bool {
    let mut tool = Bash::default();
    tool.set_protect(Some(Arc::new(protect(patterns, dir))));
    let command = format!("cd '{}' && {command}", dir.display());
    tool.call(&json!({"command": command})).is_ok()
}

#[cfg(target_os = "macos")]
#[test]
fn bash_cannot_write_delete_or_move_a_protected_path() {
    let dir = temp_dir("bash");
    fs::create_dir_all(dir.join("keep/sub")).unwrap();
    fs::write(dir.join("keep/sub/a.rs"), "hi").unwrap();
    fs::write(dir.join("other.txt"), "x").unwrap();
    let keep = [path(&dir, "keep")];
    let keep: Vec<&str> = keep.iter().map(String::as_str).collect();
    for blocked in [
        "echo x > keep/sub/a.rs",
        "rm -rf keep",
        "mv keep moved",
        "mv keep/sub other",
        "touch keep/new",
        "cp other.txt keep/",
        "chmod 777 keep/sub/a.rs",
        "ln keep/sub/a.rs hard && echo x > hard",
        "ln -s keep/sub/a.rs soft && echo x > soft",
        "mv ../$(basename \"$PWD\") ../moved",
    ] {
        assert!(!bash_succeeds(&dir, &keep, blocked), "{blocked}");
    }
    assert_eq!(fs::read_to_string(dir.join("keep/sub/a.rs")).unwrap(), "hi");
    for allowed in [
        "echo y > new.txt",
        "mkdir d && mv d e && rmdir e",
        "cat keep/sub/a.rs",
    ] {
        assert!(bash_succeeds(&dir, &keep, allowed), "{allowed}");
    }
    assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "y\n");
}

#[cfg(target_os = "macos")]
#[test]
fn bash_cannot_write_a_protected_path_in_another_case() {
    let dir = temp_dir("case");
    fs::create_dir_all(dir.join("keep")).unwrap();
    fs::write(dir.join("keep/a.rs"), "hi").unwrap();
    let keep = [path(&dir, "keep")];
    let keep: Vec<&str> = keep.iter().map(String::as_str).collect();
    let _ = bash_succeeds(&dir, &keep, "echo x > KEEP/a.rs");
    assert_eq!(fs::read_to_string(dir.join("keep/a.rs")).unwrap(), "hi");
    let protect = protect(&keep, &dir);
    assert!(protect.check_write(&path(&dir, "KEEP/a.rs")).is_err());
    assert!(protect.check_write(&path(&dir, "KEEP/new.rs")).is_err());
}
