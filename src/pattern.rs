//! File name patterns. A deployment remembers which release asset, Actions
//! artifact or file inside a package it uses as a glob (`*` and `?`), made
//! from the name picked once with its version numbers and commit hashes
//! turned into `*`, so the next version's file matches it too.

use regex::Regex;
use std::sync::OnceLock;

/// Whether `text` matches the glob `pattern` (`*` any run, `?` one
/// character). Case matters, as it does in file names.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Iterative matching with backtracking to the last star.
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn version_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\d+(?:\.\d+)+(?:[-+.~_]?(?:alpha|beta|rc|dev|pre|nightly|snapshot)[.-]?\d*)?",
        )
        .unwrap()
    })
}

fn hash_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Commit hashes and dates: runs of 7 to 40 hex digits (with at least one
    // digit) or 8-digit dates, standing apart from other letters and digits.
    RE.get_or_init(|| Regex::new(r"(^|[^0-9A-Za-z])([0-9a-f]{7,40}|\d{8})([^0-9A-Za-z]|$)").unwrap())
}

/// A pattern for `name` that the next version's file is likely to match:
/// the version (`tag`, with or without its `v`), other dotted version
/// numbers, commit hashes and dates become `*`.
pub fn derive(name: &str, tag: Option<&str>) -> String {
    let mut s = name.to_string();
    if let Some(tag) = tag {
        let bare = tag.trim_start_matches(['v', 'V']);
        // A tag that is only a word ("nightly", "latest") is left as is:
        // the next one has the same name.
        if bare.chars().any(|c| c.is_ascii_digit()) && bare.len() >= 2 {
            s = s.replace(tag, "\u{0}").replace(bare, "\u{0}");
        }
    }
    s = version_re().replace_all(&s, "\u{0}").into_owned();
    // Hashes need a loop: neighbouring matches share their separators.
    loop {
        let next = hash_re()
            .replace_all(&s, |c: &regex::Captures| {
                let m = &c[2];
                let digits_only = m.chars().all(|ch| ch.is_ascii_digit());
                let has_digit = m.chars().any(|ch| ch.is_ascii_digit());
                // Plain numbers that are not dates, and words, stay.
                if (digits_only && m.len() != 8) || !has_digit {
                    c[0].to_string()
                } else {
                    format!("{}\u{0}{}", &c[1], &c[3])
                }
            })
            .into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let ch = if ch == '\u{0}' { '*' } else { ch };
        if ch == '*' && out.ends_with('*') {
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("Chippy-*-x86_64.AppImage", "Chippy-0.2.0-x86_64.AppImage"));
        assert!(!glob_match("Chippy-*-x86_64.AppImage", "Chippy-0.2.0-aarch64.AppImage"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("*.tar.gz", "x.tar.gz"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        assert!(glob_match("exact", "exact"));
    }

    #[test]
    fn derives() {
        assert_eq!(derive("Chippy-0.1.0-x86_64.AppImage", Some("v0.1.0")), "Chippy-*-x86_64.AppImage");
        assert_eq!(derive("tool-v1.2.3-linux.tar.gz", None), "tool-v*-linux.tar.gz");
        assert_eq!(derive("chippy-linux-x86_64.tar.gz", Some("v0.1.0")), "chippy-linux-x86_64.tar.gz");
        assert_eq!(derive("yoink-linux-3f2a9c1", None), "yoink-linux-*");
        assert_eq!(derive("build-20261119-amd64.zip", None), "build-*-amd64.zip");
        assert_eq!(derive("app_2.0.0-rc1_amd64.AppImage", None), "app_*_amd64.AppImage");
        assert_eq!(derive("nightly-x86_64.AppImage", Some("nightly")), "nightly-x86_64.AppImage");
        // The derived pattern matches the next version.
        let p = derive("Surge-XT-1.3.1-linux-x86_64.tar.gz", Some("release_xt_1.3.1"));
        assert!(glob_match(&p, "Surge-XT-1.3.4-linux-x86_64.tar.gz"), "{p}");
    }
}
