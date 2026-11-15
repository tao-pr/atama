//! Protected paths of an `autonomous` agent, from `protect=<patterns>`.
//!
//! A pattern is a glob of a path, relative to the working directory or
//! absolute. The agent cannot write a path that a pattern matches, or a
//! path below it. It also cannot delete or rename a directory above a
//! protected path, because that moves the protected path.
//!
//! - `*` matches any text in one part of the path, also a name that starts
//!   with `.`. `?` matches one character in one part.
//! - `**` as a whole part matches no part or any number of parts, as in
//!   `src/**/mod.rs`.
//! - `[abc]`, `[a-z]` and `[!abc]` match one character, as in a shell. A
//!   class can have only letters, digits, `-`, `_` and `.`.
//!
//! Two checks enforce the patterns, and both use the same regular
//! expressions (see [`Protect`]):
//!
//! - The `write_file` tool calls [`Protect::check_write`] before it writes.
//! - The `bash` tool runs its command inside `sandbox-exec` with
//!   [`Protect::sandbox_profile`], so macOS denies the write, whatever the
//!   command is. Only macOS has `sandbox-exec`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use regex::Regex;

/// The protected paths of one run.
#[derive(Debug)]
pub struct Protect {
    /// Each pattern as the job wrote it, and the regular expression that
    /// matches the protected path and the paths below it.
    rules: Vec<(String, Regex)>,
    /// The regular expressions of the directories above the protected
    /// paths, for the sandbox.
    parents: Vec<String>,
}

/// Reads the patterns of `protect=<patterns>`. Commas and new lines
/// separate the patterns. A loop block writes a list as `- <item>` lines,
/// so a leading `- ` is not part of a pattern.
///
/// Fails if there is no pattern, or a pattern is invalid (see
/// [`check_pattern`]).
pub fn parse_patterns(value: &str) -> Result<Vec<String>, String> {
    let patterns: Vec<String> = value
        .split([',', '\n'])
        .map(|item| {
            let item = item.trim();
            item.strip_prefix("- ").unwrap_or(item).trim().to_string()
        })
        .filter(|item| !item.is_empty())
        .collect();
    if patterns.is_empty() {
        return Err("`protect=` has no pattern, as in `protect=\"src/**,Cargo.toml\"`".to_string());
    }
    for pattern in &patterns {
        check_pattern(pattern)?;
    }
    Ok(patterns)
}

/// Makes sure that `pattern` is a pattern that `protect=` can use.
///
/// Fails if the pattern has a `"` or a `\`, a `**` that is not a whole part
/// of the path, a `[` without a `]` in the same part, an empty class, a
/// class with another character than letters, digits, `-`, `_` and `.`, or
/// a `..` after a part with a glob.
pub fn check_pattern(pattern: &str) -> Result<(), String> {
    let error = |reason: &str| Err(format!("the `protect` pattern `{pattern}` {reason}"));
    if pattern.contains(['"', '\\']) {
        return error("has a `\"` or a `\\`");
    }
    let mut glob = false;
    for part in pattern.split('/') {
        if part == ".." && glob {
            return error("has a `..` after a part with a glob");
        }
        if part != "**" && part.contains("**") {
            return error("has a `**` that is not a whole part of the path, as in `src/**/a.rs`");
        }
        let mut chars = part.chars();
        while let Some(c) = chars.next() {
            if c != '[' {
                continue;
            }
            let Some(class) = take_class(&mut chars) else {
                return error("has a `[` without a `]`");
            };
            let items = class.strip_prefix(['!', '^']).unwrap_or(&class);
            if items.is_empty() {
                return error("has an empty class `[]`");
            }
            if !items
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return error(
                    "has a class with another character than letters, digits, `-`, `_` and `.`",
                );
            }
        }
        glob |= is_glob(part);
    }
    let parts: Vec<String> = pattern.split('/').map(String::from).collect();
    if let Err(err) = Regex::new(&parts_regex(&parts, 0)) {
        return error(&format!("is invalid: {err}"));
    }
    Ok(())
}

impl Protect {
    /// Resolves `patterns` against `cwd`, the working directory. The fixed
    /// part of a pattern, before its first glob, becomes a real path,
    /// without symbolic links, because macOS matches the real path of a
    /// file, such as `/private/tmp` for `/tmp`.
    ///
    /// Fails if a pattern is invalid, or a real path is not UTF-8 or has a
    /// `"`.
    pub fn new(patterns: &[String], cwd: &Path) -> Result<Self, String> {
        let mut rules = Vec::new();
        let mut parents = Vec::new();
        for pattern in patterns {
            check_pattern(pattern)?;
            let (parts, fixed) = absolute_parts(pattern, cwd)?;
            let source = format!("^{}(/.*)?$", parts_regex(&parts, fixed));
            let regex = Regex::new(&source).expect("check_pattern compiled the glob parts");
            rules.push((pattern.clone(), regex));
            parents.push("^/$".to_string());
            for end in 1..parts.len() {
                parents.push(format!("^{}$", parts_regex(&parts[..end], fixed)));
                if end > fixed && parts[end - 1] == "**" {
                    break;
                }
            }
        }
        parents.sort();
        parents.dedup();
        Ok(Protect { rules, parents })
    }

    /// The patterns as the job wrote them.
    pub fn patterns(&self) -> Vec<&str> {
        self.rules
            .iter()
            .map(|(pattern, _)| pattern.as_str())
            .collect()
    }

    /// Makes sure that the agent can write `path`, relative to the working
    /// directory of the process. A symbolic link to a protected path is
    /// protected too.
    ///
    /// Fails if a pattern matches the real path, or the path cannot be
    /// resolved.
    pub fn check_write(&self, path: &str) -> Result<(), String> {
        let cwd = working_dir()?;
        let real = real_path(&cwd.join(path))?;
        let real = real.to_string_lossy();
        match self.rules.iter().find(|(_, regex)| regex.is_match(&real)) {
            Some((pattern, _)) => Err(format!(
                "`{path}` is protected by `protect={pattern}`, so the agent cannot write it"
            )),
            None => Ok(()),
        }
    }

    /// The macOS sandbox profile of the `bash` tool. It allows all, but
    /// denies writes to the protected paths and the paths below them, and
    /// writes to the directories above them, such as a rename.
    pub fn sandbox_profile(&self) -> String {
        let quoted = |sources: Vec<&str>| {
            let sources: Vec<String> = sources.iter().map(|s| format!("#\"{s}\"")).collect();
            sources.join(" ")
        };
        let paths = quoted(self.rules.iter().map(|(_, regex)| regex.as_str()).collect());
        let parents = quoted(self.parents.iter().map(String::as_str).collect());
        format!(
            "(version 1)\n(allow default)\n(deny file-write* (regex {paths}))\n\
             (deny file-write* (require-all (vnode-type DIRECTORY) (regex {parents})))\n"
        )
    }

    /// Tells the model which paths it cannot write.
    pub fn instruction(&self) -> String {
        let patterns: Vec<String> = self.patterns().iter().map(|p| format!("`{p}`")).collect();
        format!(
            "Protected paths. You cannot write, delete or rename these paths, the paths below \
             them, or the directories above them. A tool call that tries fails: {}",
            patterns.join(", ")
        )
    }
}

/// Takes the characters of a class after its `[`, and its `]`. Returns
/// `None` if the class has no `]`.
fn take_class(chars: &mut std::str::Chars) -> Option<String> {
    let mut class = String::new();
    for c in chars.by_ref() {
        if c == ']' {
            return Some(class);
        }
        class.push(c);
    }
    None
}

/// The working directory of the process.
///
/// Fails if it cannot be read.
pub fn working_dir() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir();
    cwd.map_err(|err| format!("cannot read the working directory: {err}"))
}

/// Returns true if `part` has a glob character.
fn is_glob(part: &str) -> bool {
    part.contains(['*', '?', '['])
}

/// The parts of the absolute pattern of `pattern`, without `.`, and with
/// `..` applied, and the number of fixed parts. The fixed parts, before the
/// first glob, are a real path. The parts of `cwd` are plain text, also if
/// they have a glob character.
fn absolute_parts(pattern: &str, cwd: &Path) -> Result<(Vec<String>, usize), String> {
    let mut parts: Vec<String> = Vec::new();
    if !pattern.starts_with('/') {
        let cwd = cwd.to_string_lossy();
        parts.extend(
            cwd.split('/')
                .filter(|part| !part.is_empty())
                .map(String::from),
        );
    }
    // The number of parts from `cwd`, which are plain text.
    let mut plain = parts.len();
    for part in pattern.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
                plain = plain.min(parts.len());
            }
            part => parts.push(part.to_string()),
        }
    }
    let glob = (plain..parts.len())
        .find(|&index| is_glob(&parts[index]))
        .unwrap_or(parts.len());
    let real = real_path(&PathBuf::from(format!("/{}", parts[..glob].join("/"))))?;
    let Some(real) = real.to_str().filter(|real| !real.contains('"')) else {
        return Err(format!(
            "the real path `{}` of the `protect` pattern `{pattern}` is not UTF-8 or has a `\"`",
            real.display()
        ));
    };
    let mut absolute: Vec<String> = real
        .split('/')
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect();
    let fixed = absolute.len();
    absolute.extend(parts.drain(glob..));
    Ok((absolute, fixed))
}

/// The real path of `path`, without symbolic links. The parts that do not
/// exist yet stay as they are.
///
/// Fails if a `..` follows a directory that does not exist.
fn real_path(path: &Path) -> Result<PathBuf, String> {
    let mut existing = path.to_path_buf();
    let mut missing: Vec<OsString> = Vec::new();
    let mut real = loop {
        if let Ok(real) = existing.canonicalize() {
            break real;
        }
        match existing.file_name() {
            Some(name) => missing.push(name.to_owned()),
            None => {
                return Err(format!(
                    "cannot resolve `{}`, because a `..` follows a directory that does not exist",
                    path.display()
                ));
            }
        }
        existing.pop();
    };
    real.extend(missing.iter().rev());
    Ok(real)
}

/// The regular expression of the absolute path `parts`, without `^` and
/// `$`. Each part adds `/` and the part, and `**` adds no part or any parts.
/// The first `fixed` parts are plain text, without globs.
fn parts_regex(parts: &[String], fixed: usize) -> String {
    let mut regex = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index < fixed {
            regex.push('/');
            for c in part.chars() {
                push_plain(&mut regex, c);
            }
            continue;
        }
        if part == "**" {
            regex.push_str("(/.*)?");
            continue;
        }
        regex.push('/');
        let mut chars = part.chars();
        while let Some(c) = chars.next() {
            match c {
                '*' => regex.push_str("[^/]*"),
                '?' => regex.push_str("[^/]"),
                '[' => {
                    let class = take_class(&mut chars).unwrap_or_default();
                    let class = match class.strip_prefix(['!', '^']) {
                        Some(items) => format!("^{items}"),
                        None => class,
                    };
                    regex.push_str(&format!("[{class}]"));
                }
                c => push_plain(&mut regex, c),
            }
        }
    }
    regex
}

/// Adds the character `c` to `regex` as plain text.
fn push_plain(regex: &mut String, c: char) {
    if ".^$*+?()[]{}|".contains(c) {
        regex.push('\\');
    }
    regex.push(c);
}
