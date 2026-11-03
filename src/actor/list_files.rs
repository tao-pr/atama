//! The `list_files` action: lists the files of one directory whose names
//! match a glob pattern. It does not go into subdirectories.

use std::fs;
use std::path::Path;

/// The pattern of a `list_files` action without `pattern=`.
pub const DEFAULT_PATTERN: &str = "*";

/// Makes sure that `pattern` is a glob pattern that `list_files` can use.
///
/// Fails if the pattern is empty, has a `/` (the action does not go into
/// subdirectories), or has a `[` without a closing `]`.
pub fn check_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() {
        return Err("the `list_files` pattern is empty".to_string());
    }
    if pattern.contains('/') {
        return Err(format!(
            "the `list_files` pattern `{pattern}` has a `/`, but `list_files` does not \
             go into subdirectories"
        ));
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '[' => match class_end(&chars, i) {
                Some(end) => i = end + 1,
                None => {
                    return Err(format!(
                        "the `list_files` pattern `{pattern}` has a `[` without a closing `]`"
                    ));
                }
            },
            _ => i += 1,
        }
    }
    Ok(())
}

/// Returns true if the file name `name` matches the glob `pattern`.
///
/// - `*` matches any text, also no text.
/// - `?` matches one character.
/// - `[abc]` matches one of the characters, and `[a-z]` one in the range.
///   `[!abc]` or `[^abc]` matches one character that is not in the class.
/// - `\` makes the next character a plain character, as in `\*`.
/// - A name that starts with `.` matches only a pattern that starts with
///   `.`, as in a shell. So `*` does not match `.env`, but `.*` does.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0, 0);
    // The position after the last `*`, and the name position it matched up to.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p] == '*' {
            p += 1;
            star = Some((p, n));
            continue;
        }
        if let Some(next) = match_one(&pattern, p, name[n]) {
            p = next;
            n += 1;
            continue;
        }
        match star {
            // Let the last `*` match one more character, and try again.
            Some((star_p, star_n)) => {
                p = star_p;
                n = star_n + 1;
                star = Some((star_p, star_n + 1));
            }
            None => return false,
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Matches the character `c` with the pattern item at `p`. Returns the
/// position of the next pattern item, or `None` if `c` does not match.
fn match_one(pattern: &[char], p: usize, c: char) -> Option<usize> {
    match *pattern.get(p)? {
        '?' => Some(p + 1),
        '\\' if p + 1 < pattern.len() => (pattern[p + 1] == c).then_some(p + 2),
        '[' => match class_end(pattern, p) {
            Some(end) => class_matches(&pattern[p + 1..end], c).then_some(end + 1),
            // A `[` without `]` is a plain character.
            None => (c == '[').then_some(p + 1),
        },
        plain => (plain == c).then_some(p + 1),
    }
}

/// Returns the position of the `]` that closes the class at `start`. A `]`
/// just after `[`, `[!` or `[^` is part of the class.
fn class_end(pattern: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    if matches!(pattern.get(i), Some('!' | '^')) {
        i += 1;
    }
    if pattern.get(i) == Some(&']') {
        i += 1;
    }
    (i..pattern.len()).find(|&i| pattern[i] == ']')
}

/// Returns true if `c` is in the class `class`, the text between `[` and `]`.
fn class_matches(class: &[char], c: char) -> bool {
    let (negated, class) = match class.first() {
        Some('!' | '^') => (true, &class[1..]),
        _ => (false, class),
    };
    let mut found = false;
    let mut i = 0;
    while i < class.len() {
        if i + 2 < class.len() && class[i + 1] == '-' {
            found |= class[i] <= c && c <= class[i + 2];
            i += 3;
        } else {
            found |= class[i] == c;
            i += 1;
        }
    }
    found != negated
}

/// Lists the files in the directory `dir`, or in the directory in `input` if
/// `dir` is `None`, whose names match `pattern` (see [`glob_match`]). It does
/// not go into subdirectories, and it lists only files, not directories. A
/// symbolic link to a file is a file.
///
/// Returns one path per line, sorted by name, and each line ends with a
/// newline. Each path is the directory joined with the file name, so that
/// `foreach read_file` can read the files. A directory without matching
/// files gives an empty output.
///
/// Fails if `dir` is `None` and `input` has no path or more than one line,
/// or the directory cannot be read, or a file name is not UTF-8.
pub(super) fn list_files(dir: Option<&str>, pattern: &str, input: &str) -> Result<String, String> {
    let dir = match dir {
        Some(dir) => dir,
        None => {
            let dir = input.trim();
            if dir.is_empty() {
                return Err(
                    "`list_files` needs a directory in `dir=` or in its input".to_string(),
                );
            }
            if dir.lines().count() > 1 {
                return Err(format!(
                    "`list_files` lists one directory, but its input has {} lines. To list \
                     one directory for each line, use `foreach list_files`",
                    dir.lines().count()
                ));
            }
            dir
        }
    };
    let entries =
        fs::read_dir(dir).map_err(|err| format!("cannot list directory `{dir}`: {err}"))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot list directory `{dir}`: {err}"))?;
        let name = entry.file_name().into_string().map_err(|name| {
            format!(
                "cannot list directory `{dir}`: the file name `{}` is not UTF-8",
                name.to_string_lossy()
            )
        })?;
        if !glob_match(pattern, &name) {
            continue;
        }
        // `fs::metadata` follows symbolic links.
        if fs::metadata(entry.path()).is_ok_and(|meta| meta.is_file()) {
            names.push(name);
        }
    }
    names.sort();
    let mut output = String::new();
    for name in names {
        output.push_str(&Path::new(dir).join(name).to_string_lossy());
        output.push('\n');
    }
    Ok(output)
}
