//! The canonical library layout — the single folder shape the normalizer
//! moves toward and the release builder writes from day one (so releases
//! built in Plinth stop future designers drifting into ad-hoc structures):
//!
//! ```text
//! {root}/{Designer}/{YYYY-MM Release}/{Model}/{Supported|Unsupported}/[{variant}/]files
//! ```
//!
//! Poses are deliberately NOT folders: a pose is metadata (file_variants /
//! model.json file_poses) and shows up in file NAMES, so a whole pose set
//! prints in one multi-select without folder-diving. Everything here is
//! pure path math — no filesystem, no database — so it unit-tests exactly.

use std::path::{Path, PathBuf};

/// Make a name safe as a single path segment on every OS we ship to.
/// Windows is the strict one: reserved characters, trailing dots/spaces,
/// and legacy device names are all invalid there but fine on macOS/Linux —
/// and the library lives on a NAS serving both.
pub fn sanitize_segment(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if (c as u32) < 0x20 => ' ',
            c => c,
        })
        .collect();
    // collapse runs of whitespace the replacements may have created
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(['.', ' ']).to_string();
    if trimmed.is_empty() {
        return "Unnamed".to_string();
    }
    // CON, PRN, AUX, NUL, COM1-9, LPT1-9 (with or without an extension)
    // are unusable file names on Windows to this day
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        format!("{}_", trimmed)
    } else {
        trimmed
    }
}

/// "M/YYYY" (release builder) or "YYYY-MM" (scanner-derived) -> "YYYY-MM".
/// Anything else is unusable for sorting and returns None.
fn sortable_date(date: &str) -> Option<String> {
    let date = date.trim();
    if let Some((month, year)) = date.split_once('/') {
        let month: u32 = month.trim().parse().ok()?;
        let year: u32 = year.trim().parse().ok()?;
        if (1..=12).contains(&month) && year >= 1000 {
            return Some(format!("{:04}-{:02}", year, month));
        }
    }
    if let Some((year, month)) = date.split_once('-') {
        let year: u32 = year.trim().parse().ok()?;
        let month: u32 = month.trim().parse().ok()?;
        if (1..=12).contains(&month) && year >= 1000 {
            return Some(format!("{:04}-{:02}", year, month));
        }
    }
    None
}

/// The release-level folder: "2026-07 Dread Swamp". The date prefix makes
/// releases sort chronologically in Finder/Explorer; a dateless release is
/// just its name rather than being blocked on missing metadata.
pub fn release_segment(release_name: &str, release_date: Option<&str>) -> String {
    let name = sanitize_segment(release_name);
    match release_date.and_then(sortable_date) {
        Some(date) => format!("{} {}", date, name),
        None => name,
    }
}

/// The casing CONVENTION for variant names: Title Case, tool-decided.
/// "sword", "SWORD" and "Sword" are one variant — letting whoever typed
/// first pick the spelling made grouping depend on data-entry accidents.
/// Acronym-styled variants ("OPR") flatten to "Opr"; that's the price of
/// a convention and it's consistent.
pub fn title_case(name: &str) -> String {
    name.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "supported"/"presupported" -> Supported, "unsupported" -> Unsupported.
/// Anything else (None, unknown strings) has no canonical build folder.
pub fn support_segment(support: Option<&str>) -> Option<&'static str> {
    match support?.to_ascii_lowercase().as_str() {
        "supported" | "presupported" => Some("Supported"),
        "unsupported" => Some("Unsupported"),
        _ => None,
    }
}

/// The canonical dir for one MODEL (the group card):
/// root/Designer/[YYYY-MM Release/]Model. A release-less model sits
/// directly under its designer.
pub fn model_dir(
    root: &Path,
    designer: &str,
    release_name: Option<&str>,
    release_date: Option<&str>,
    model_name: &str,
) -> PathBuf {
    let mut dir = root.join(sanitize_segment(designer));
    if let Some(release) = release_name.filter(|r| !r.trim().is_empty()) {
        dir = dir.join(release_segment(release, release_date));
    }
    dir.join(sanitize_segment(model_name))
}

/// The canonical leaf dir for one MEMBER (a support build, optionally a
/// variant of it) inside its model dir. Unknown support keeps the files at
/// the model root — inventing a build folder from missing data would just
/// be a different kind of mess.
pub fn member_dir(model_dir: &Path, support: Option<&str>, variant: Option<&str>) -> PathBuf {
    match support_segment(support) {
        Some(build) => {
            let dir = model_dir.join(build);
            match variant.filter(|v| !v.trim().is_empty()) {
                // variant folders always carry the conventional casing
                Some(v) => dir.join(sanitize_segment(&title_case(v))),
                None => dir,
            }
        }
        // no build folder means no variant tier either — variant without
        // support is not a shape the canon defines
        None => model_dir.to_path_buf(),
    }
}

/// Bake a pose into a file name when pose dirs merge into one build folder:
/// "galeb duhr.stl" + "A" -> "galeb duhr A.stl". A name that already ends
/// with the pose token stays untouched, so re-running the normalizer never
/// grows "galeb duhr A A.stl".
pub fn pose_suffixed_name(file_name: &str, pose: &str) -> String {
    let pose = pose.trim();
    if pose.is_empty() {
        return file_name.to_string();
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (file_name, None),
    };
    let already_suffixed = stem
        .to_lowercase()
        .ends_with(&format!(" {}", pose.to_lowercase()))
        || stem.eq_ignore_ascii_case(pose);
    let new_stem = if already_suffixed {
        stem.to_string()
    } else {
        format!("{} {}", stem, sanitize_segment(pose))
    };
    match ext {
        Some(ext) => format!("{}.{}", new_stem, ext),
        None => new_stem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_makes_windows_safe_segments() {
        assert_eq!(sanitize_segment("Bog: Hag?"), "Bog Hag");
        assert_eq!(sanitize_segment("a/b\\c"), "a b c");
        assert_eq!(sanitize_segment("  spaced.  "), "spaced");
        assert_eq!(sanitize_segment("trailing dots..."), "trailing dots");
        assert_eq!(sanitize_segment(""), "Unnamed");
        assert_eq!(sanitize_segment("***"), "Unnamed");
        // Windows device names get a defusing underscore
        assert_eq!(sanitize_segment("CON"), "CON_");
        assert_eq!(sanitize_segment("com1"), "com1_");
        // ...but only exact device names; lookalikes pass through
        assert_eq!(sanitize_segment("CONAN"), "CONAN");
        assert_eq!(sanitize_segment("COM10"), "COM10");
    }

    #[test]
    fn release_segment_prefixes_a_sortable_date() {
        // both date spellings in the wild normalize to YYYY-MM
        assert_eq!(
            release_segment("Dread Swamp", Some("7/2026")),
            "2026-07 Dread Swamp"
        );
        assert_eq!(
            release_segment("Dread Swamp", Some("2026-07")),
            "2026-07 Dread Swamp"
        );
        // garbage dates degrade to the bare name, never block
        assert_eq!(release_segment("Dread Swamp", Some("july")), "Dread Swamp");
        assert_eq!(release_segment("Dread Swamp", None), "Dread Swamp");
    }

    #[test]
    fn model_dir_builds_the_canonical_tree() {
        let root = Path::new("/lib");
        assert_eq!(
            model_dir(root, "Bestiarum", Some("Dread Swamp"), Some("7/2026"), "Bog Hag"),
            Path::new("/lib/Bestiarum/2026-07 Dread Swamp/Bog Hag")
        );
        // release-less models sit directly under the designer
        assert_eq!(
            model_dir(root, "Bestiarum", None, Some("7/2026"), "Bog Hag"),
            Path::new("/lib/Bestiarum/Bog Hag")
        );
    }

    #[test]
    fn member_dir_places_builds_and_variants() {
        let model = Path::new("/lib/B/R/Bog Hag");
        assert_eq!(
            member_dir(model, Some("supported"), None),
            Path::new("/lib/B/R/Bog Hag/Supported")
        );
        assert_eq!(
            member_dir(model, Some("presupported"), Some("sword")),
            Path::new("/lib/B/R/Bog Hag/Supported/Sword")
        );
        assert_eq!(
            member_dir(model, Some("unsupported"), None),
            Path::new("/lib/B/R/Bog Hag/Unsupported")
        );
        // unknown support -> model root, variant tier does not apply
        assert_eq!(member_dir(model, None, Some("sword")), model);
    }

    #[test]
    fn title_case_is_the_variant_convention() {
        assert_eq!(title_case("sword"), "Sword");
        assert_eq!(title_case("SWORD"), "Sword");
        assert_eq!(title_case("great swords"), "Great Swords");
        assert_eq!(title_case("sword + flower shield"), "Sword + Flower Shield");
        assert_eq!(title_case("  spaced   out "), "Spaced Out");
        // the convention flattens acronyms — consistent beats clever
        assert_eq!(title_case("OPR"), "Opr");
    }

    #[test]
    fn pose_suffixing_is_idempotent() {
        assert_eq!(pose_suffixed_name("galeb duhr.stl", "A"), "galeb duhr A.stl");
        assert_eq!(pose_suffixed_name("galeb duhr A.stl", "A"), "galeb duhr A.stl");
        assert_eq!(pose_suffixed_name("galeb duhr a.stl", "A"), "galeb duhr a.stl");
        // extensionless and dotfile-ish names survive
        assert_eq!(pose_suffixed_name("README", "A"), "README A");
        assert_eq!(pose_suffixed_name(".hidden", "A"), ".hidden A");
        // a file NAMED like the pose gains nothing
        assert_eq!(pose_suffixed_name("A.stl", "A"), "A.stl");
    }
}
