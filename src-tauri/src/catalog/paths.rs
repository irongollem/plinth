//! Lexical path rules for catalog roots and the containment checks built
//! on them.
//!
//! Lexical, not filesystem: these run against strings the scanner wrote
//! into the database, strings a settings file carries between machines,
//! and strings a picker handed us. `std::path` would answer for the host
//! instead — its separator, its idea of a root — and a Windows path
//! examined on any other platform gets the wrong answer from it.

/// Either spelling counts. Windows accepts both, and a catalog written on
/// Windows can be read anywhere.
fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Length of the filesystem root that `path` starts with, in bytes.
///
/// The three shapes that matter: a POSIX `/`, a Windows drive (`Z:` or
/// `Z:\`), and a UNC share (`\\server\share`). Everything else — relative
/// paths included — has no root.
fn root_len(path: &str) -> Option<usize> {
    let bytes = path.as_bytes();
    let sep_at = |i: usize| bytes.get(i).is_some_and(|b| is_sep(*b as char));

    // UNC: \\server\share — the share is part of the root, so \\server on
    // its own is not one. An incomplete spelling falls back to the leading
    // separator, keeping "every absolute path has a root" true.
    if sep_at(0) && sep_at(1) {
        let server_start = 2;
        if let Some(server_end) = (server_start..bytes.len()).find(|i| sep_at(*i)) {
            let share_start = server_end + 1;
            let share_end = (share_start..bytes.len())
                .find(|i| sep_at(*i))
                .unwrap_or(bytes.len());
            if server_end > server_start && share_end > share_start {
                return Some(share_end);
            }
        }
        return Some(1);
    }

    // Drive: a bare "Z:" is drive-RELATIVE on Windows ("the current
    // directory on Z"), so it is reported as a 2-byte root that
    // normalize_root then completes.
    if bytes.len() >= 2 && (bytes[0] as char).is_ascii_alphabetic() && bytes[1] == b':' {
        return Some(if sep_at(2) { 3 } else { 2 });
    }

    if sep_at(0) {
        return Some(1);
    }
    None
}

/// The canonical spelling of a catalog root: redundant trailing
/// separators gone, the root itself left absolute.
///
/// The distinction is the whole point. `Z:\Designer\` and `Z:\Designer`
/// name the same folder, but `Z:\` and `Z:` do not: the second is
/// drive-relative, resolving against whatever the process last set as the
/// current directory on Z. Trimming trailing separators blindly turns an
/// absolute root into that, so a persisted root could come back pointing
/// somewhere else entirely. A bare drive is completed rather than left
/// alone, which also repairs the values written by versions that trimmed.
pub fn normalize_root(path: &str) -> String {
    match root_len(path) {
        // "Z:" — give the root its separator back
        Some(2) if path.len() == 2 && path.as_bytes()[1] == b':' => format!("{}\\", path),
        Some(root) => {
            let mut end = path.len();
            while end > root && matches!(path.as_bytes()[end - 1], b'/' | b'\\') {
                end -= 1;
            }
            path[..end].to_string()
        }
        None => {
            let trimmed = path.trim_end_matches(is_sep);
            if trimmed.is_empty() {
                path.to_string()
            } else {
                trimmed.to_string()
            }
        }
    }
}

/// `child` is `base` itself or lies beneath it, by path segment —
/// `/lib/ab` is not under `/lib/a`.
///
/// Both sides are normalized first, so a trailing separator on either
/// changes nothing, and a root base (`Z:\`, `/`) contains its children
/// even though it already ends in a separator.
pub fn is_under(child: &str, base: &str) -> bool {
    let child = normalize_root(child);
    let base = normalize_root(base);
    if child == base {
        return true;
    }
    let Some(rest) = child.strip_prefix(&base) else {
        return false;
    };
    // A root that already ends in its separator (`Z:\`, `/`) has no
    // separator left to find; a share root (`\\nas\models`) does, and
    // without that check `\\nas\modelsOld` would count as inside it.
    if base.ends_with(is_sep) {
        !rest.is_empty()
    } else {
        rest.starts_with(is_sep)
    }
}

/// The literal prefix that means "inside this root", for the SQL scoping
/// that cannot call back into Rust.
///
/// Two things the old `root || MAIN_SEPARATOR` got wrong. A drive or share
/// root already ends in its separator, so appending one produced `Z:\\`,
/// which matches nothing — every child of a drive root fell outside its
/// own root. And the separator came from the host rather than from the
/// path, so a catalog scanned on Windows could not be scoped anywhere
/// else, which is what made the Windows cases untestable.
pub fn child_prefix(root: &str) -> String {
    let root = normalize_root(root);
    if root.ends_with(is_sep) {
        return root;
    }
    let sep = root
        .rfind(is_sep)
        .map(|i| root[i..i + 1].to_string())
        .unwrap_or_else(|| std::path::MAIN_SEPARATOR.to_string());
    format!("{}{}", root, sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_root_stays_absolute() {
        // The bug: trimming the separator leaves "Z:", which Windows reads
        // as "wherever the process last was on Z".
        assert_eq!(normalize_root("Z:\\"), "Z:\\");
        assert_eq!(normalize_root("Z:"), "Z:\\");
        assert_eq!(normalize_root("Z:\\\\"), "Z:\\");
        assert_eq!(normalize_root("Z:/"), "Z:/");
    }

    #[test]
    fn nested_paths_lose_only_their_trailing_separator() {
        assert_eq!(normalize_root("Z:\\Designer\\"), "Z:\\Designer");
        assert_eq!(normalize_root("Z:\\Designer"), "Z:\\Designer");
        assert_eq!(normalize_root("/library/"), "/library");
        assert_eq!(normalize_root("/library"), "/library");
        assert_eq!(normalize_root("/library/sub///"), "/library/sub");
    }

    #[test]
    fn posix_and_unc_roots_survive_normalization() {
        assert_eq!(normalize_root("/"), "/");
        assert_eq!(normalize_root("//"), "/");
        assert_eq!(normalize_root("\\\\nas\\models"), "\\\\nas\\models");
        assert_eq!(normalize_root("\\\\nas\\models\\"), "\\\\nas\\models");
        assert_eq!(
            normalize_root("\\\\nas\\models\\dtl\\"),
            "\\\\nas\\models\\dtl"
        );
    }

    #[test]
    fn relative_paths_are_left_relative() {
        assert_eq!(normalize_root("models/"), "models");
        assert_eq!(normalize_root("models"), "models");
        assert_eq!(normalize_root(""), "");
    }

    #[test]
    fn containment_is_segment_aware_under_every_root_shape() {
        assert!(is_under("Z:\\Designer", "Z:\\"));
        assert!(is_under("Z:\\Designer\\Set", "Z:\\Designer"));
        // the drive-relative spelling means the same root once repaired
        assert!(is_under("Z:\\Designer", "Z:"));
        assert!(is_under("/library/dtl", "/"));
        assert!(is_under("/library/dtl", "/library/"));
        assert!(is_under("\\\\nas\\models\\dtl", "\\\\nas\\models"));

        // a shared prefix that isn't a segment boundary
        assert!(!is_under("/library/ab", "/library/a"));
        assert!(!is_under("Z:\\Designers", "Z:\\Designer"));
        // a share root ends in no separator of its own, so a sibling share
        // must not read as one of its children
        assert!(!is_under("\\\\nas\\modelsOld", "\\\\nas\\models"));
        // and a different drive entirely
        assert!(!is_under("Y:\\Designer", "Z:\\"));
    }

    #[test]
    fn child_prefix_never_doubles_a_root_separator() {
        // "Z:\" + "\" is "Z:\\", which matches no path at all
        assert_eq!(child_prefix("Z:\\"), "Z:\\");
        assert_eq!(child_prefix("Z:"), "Z:\\");
        assert_eq!(child_prefix("/"), "/");
        assert_eq!(child_prefix("\\\\nas\\models"), "\\\\nas\\models\\");
    }

    #[test]
    fn child_prefix_takes_its_separator_from_the_path_not_the_host() {
        // both of these have to hold on every platform, or a Windows
        // catalog can only be scoped on Windows
        assert_eq!(child_prefix("Z:\\Designer"), "Z:\\Designer\\");
        assert_eq!(child_prefix("/library"), "/library/");
        assert_eq!(child_prefix("/library/"), "/library/");
    }
}
