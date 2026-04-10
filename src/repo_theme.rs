use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::types::RepoTheme;

const PREFERRED_THEME_DIR: &str = ".swimmers";
const LEGACY_THEME_DIR: &str = ".throngterm";
const DEFAULT_SKIN: &str = "#F3D2B6";
const DEFAULT_TAN: &str = "#DCCAB6";
const DEFAULT_INK: &str = "#171717";
const MOTIFS: &[&str] = &[
    "spark", "terminal", "hammer", "wave", "comet", "gear", "leaf", "bolt",
];

#[derive(Debug, Deserialize)]
struct ColorsFileInput {
    sprite: Option<String>,
    palette: Option<PaletteInput>,
}

#[derive(Debug, Deserialize)]
struct PaletteInput {
    body: Option<String>,
    outline: Option<String>,
    accent: Option<String>,
    shirt: Option<String>,
}

#[derive(Debug, Serialize)]
struct ColorsFileOutput {
    target: String,
    generated_at: String,
    palette: PaletteOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    sprite: Option<String>,
    motif: String,
}

#[derive(Debug, Serialize)]
struct PaletteOutput {
    body: String,
    outline: String,
    accent: String,
    shirt: String,
    skin: String,
    tan: String,
    ink: String,
    motif: String,
}

pub fn existing_repo_theme(cwd: &str) -> Option<(String, RepoTheme)> {
    let project_root = walk_to_theme_root(cwd)?;
    let theme = read_first_valid_theme(&project_root)?;
    Some((project_root.to_string_lossy().into_owned(), theme))
}

pub fn discover_repo_theme(cwd: &str) -> Option<(String, RepoTheme)> {
    let project_root = walk_to_theme_root(cwd)?;
    let colors_path = preferred_colors_path(&project_root);

    if let Some(existing) = existing_repo_theme(cwd) {
        return Some(existing);
    }

    let used_colors = collect_used_colors(&project_root);
    let theme = generate_unique_theme(&project_root, &used_colors);
    if let Err(err) = write_theme_file(&project_root, &colors_path, &theme) {
        tracing::warn!(
            project_root = %project_root.to_string_lossy(),
            path = %colors_path.to_string_lossy(),
            "failed to persist generated repo theme: {}",
            err
        );
    }

    Some((project_root.to_string_lossy().into_owned(), theme))
}

fn walk_to_theme_root(cwd: &str) -> Option<PathBuf> {
    let mut current = Path::new(cwd);
    if !current.is_dir() {
        current = current.parent()?;
    }

    loop {
        if has_theme_dir(current) {
            return Some(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return None,
        }
    }
}

fn has_theme_dir(project_root: &Path) -> bool {
    theme_dir_paths(project_root)
        .into_iter()
        .any(|path| path.is_dir())
}

fn theme_dir_paths(project_root: &Path) -> [PathBuf; 2] {
    [
        project_root.join(PREFERRED_THEME_DIR),
        project_root.join(LEGACY_THEME_DIR),
    ]
}

fn theme_file_paths(project_root: &Path) -> [PathBuf; 2] {
    theme_dir_paths(project_root).map(|path| path.join("colors.json"))
}

fn preferred_colors_path(project_root: &Path) -> PathBuf {
    project_root.join(PREFERRED_THEME_DIR).join("colors.json")
}

fn read_first_valid_theme(project_root: &Path) -> Option<RepoTheme> {
    theme_file_paths(project_root)
        .into_iter()
        .find_map(|path| read_valid_theme(&path))
}

fn read_valid_theme(path: &Path) -> Option<RepoTheme> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: ColorsFileInput = serde_json::from_str(&raw).ok()?;
    let palette = parsed.palette?;
    Some(RepoTheme {
        body: normalize_hex(palette.body.as_deref()?)?,
        outline: normalize_hex(palette.outline.as_deref()?)?,
        accent: normalize_hex(palette.accent.as_deref()?)?,
        shirt: normalize_hex(palette.shirt.as_deref()?)?,
        sprite: parsed.sprite.as_deref().and_then(normalize_sprite_name),
    })
}

fn collect_used_colors(project_root: &Path) -> HashSet<String> {
    let mut used = HashSet::new();
    let Some(parent) = project_root.parent() else {
        return used;
    };

    let Ok(entries) = std::fs::read_dir(parent) else {
        return used;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path == project_root {
            continue;
        }
        let Some(theme) = read_first_valid_theme(&path) else {
            continue;
        };
        used.insert(theme.body);
        used.insert(theme.outline);
        used.insert(theme.accent);
        used.insert(theme.shirt);
    }

    used
}

fn generate_unique_theme(project_root: &Path, used_colors: &HashSet<String>) -> RepoTheme {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    project_root.to_string_lossy().hash(&mut hasher);
    let seed = hasher.finish();

    for attempt in 0..2048_u64 {
        let candidate = theme_candidate(seed, attempt);
        if candidate.is_distinct(used_colors) {
            return candidate;
        }
    }

    theme_candidate(seed, 4096)
}

fn theme_candidate(seed: u64, attempt: u64) -> RepoTheme {
    let orbit = seed.wrapping_add(attempt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let hue = wrap_hue(seed_fraction(orbit) * 360.0 + attempt as f64 * 137.507_764);
    let body_s = 0.52 + seed_fraction(orbit.rotate_left(11)) * 0.16;
    let body_l = 0.44 + seed_fraction(orbit.rotate_left(23)) * 0.16;
    let shirt_hue = wrap_hue(hue + 40.0 + seed_fraction(orbit.rotate_left(7)) * 110.0);
    let shirt_s = 0.28 + seed_fraction(orbit.rotate_left(29)) * 0.18;
    let shirt_l = 0.50 + seed_fraction(orbit.rotate_left(37)) * 0.14;

    RepoTheme {
        body: rgb_to_hex(hsl_to_rgb(hue, body_s, body_l)),
        outline: rgb_to_hex(hsl_to_rgb(
            wrap_hue(hue + 6.0),
            (body_s * 0.58).clamp(0.22, 0.52),
            (body_l * 0.36).clamp(0.14, 0.26),
        )),
        accent: rgb_to_hex(hsl_to_rgb(
            wrap_hue(hue - 10.0),
            (body_s * 0.72).clamp(0.30, 0.62),
            (body_l * 0.22).clamp(0.08, 0.18),
        )),
        shirt: rgb_to_hex(hsl_to_rgb(shirt_hue, shirt_s, shirt_l)),
        sprite: None,
    }
}

fn write_theme_file(
    project_root: &Path,
    colors_path: &Path,
    theme: &RepoTheme,
) -> std::io::Result<()> {
    if let Some(parent) = colors_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let output = ColorsFileOutput {
        target: project_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string()),
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        palette: PaletteOutput {
            body: theme.body.clone(),
            outline: theme.outline.clone(),
            accent: theme.accent.clone(),
            shirt: theme.shirt.clone(),
            skin: DEFAULT_SKIN.to_string(),
            tan: DEFAULT_TAN.to_string(),
            ink: DEFAULT_INK.to_string(),
            motif: rgb_to_hex(hsl_to_rgb(
                wrap_hue(hex_hue(&theme.shirt).unwrap_or(210.0) + 28.0),
                0.58,
                0.60,
            )),
        },
        sprite: theme.sprite.as_deref().and_then(normalize_sprite_name),
        motif: motif_name(project_root),
    };

    let json = serde_json::to_string_pretty(&output)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    std::fs::write(colors_path, format!("{json}\n"))
}

fn motif_name(project_root: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    project_root.to_string_lossy().hash(&mut hasher);
    let idx = (hasher.finish() as usize) % MOTIFS.len();
    MOTIFS[idx].to_string()
}

fn normalize_hex(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() != 7 || !trimmed.starts_with('#') {
        return None;
    }
    let mut normalized = String::with_capacity(7);
    normalized.push('#');
    for ch in trimmed[1..].chars() {
        if !ch.is_ascii_hexdigit() {
            return None;
        }
        normalized.push(ch.to_ascii_uppercase());
    }
    Some(normalized)
}

fn normalize_sprite_name(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fish" => Some("fish".to_string()),
        "balls" => Some("balls".to_string()),
        "jelly" => Some("jelly".to_string()),
        _ => None,
    }
}

fn seed_fraction(seed: u64) -> f64 {
    let bucket = (seed % 10_000) as f64;
    bucket / 10_000.0
}

fn wrap_hue(hue: f64) -> f64 {
    let wrapped = hue % 360.0;
    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = wrap_hue(h) / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime {
        hp if hp < 1.0 => (c, x, 0.0),
        hp if hp < 2.0 => (x, c, 0.0),
        hp if hp < 3.0 => (0.0, c, x),
        hp if hp < 4.0 => (0.0, x, c),
        hp if hp < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_byte = |value: f64| ((value + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to_byte(r1), to_byte(g1), to_byte(b1))
}

fn rgb_to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn hex_hue(hex: &str) -> Option<f64> {
    let value = normalize_hex(hex)?;
    let r = u8::from_str_radix(&value[1..3], 16).ok()? as f64 / 255.0;
    let g = u8::from_str_radix(&value[3..5], 16).ok()? as f64 / 255.0;
    let b = u8::from_str_radix(&value[5..7], 16).ok()? as f64 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta == 0.0 {
        return Some(0.0);
    }

    let hue = if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    Some(wrap_hue(hue))
}

impl RepoTheme {
    fn is_distinct(&self, used_colors: &HashSet<String>) -> bool {
        let values = [&self.body, &self.outline, &self.accent, &self.shirt];
        let unique: HashSet<&String> = values.iter().copied().collect();
        if unique.len() != values.len() {
            return false;
        }
        values.iter().all(|value| !used_colors.contains(*value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn write_theme_file(path: &Path, dir_name: &str, body: &str) {
        write(
            &path.join(dir_name).join("colors.json"),
            &format!(
                r##"{{
  "palette": {{
    "body": "{body}",
    "outline": "#3D2F24",
    "accent": "#1D1914",
    "shirt": "#AA9370"
  }}
}}"##
            ),
        );
    }

    #[test]
    fn reads_existing_valid_theme() {
        let tmp = tempfile::tempdir().unwrap();
        let colors_path = tmp.path().join(PREFERRED_THEME_DIR).join("colors.json");
        write(
            &colors_path,
            r##"{
  "palette": {
    "body": "#b89875",
    "outline": "#3d2f24",
    "accent": "#1d1914",
    "shirt": "#aa9370"
  }
}"##,
        );

        let cwd = tmp.path().join("src");
        std::fs::create_dir_all(&cwd).unwrap();
        let (root, theme) = discover_repo_theme(cwd.to_string_lossy().as_ref()).unwrap();

        assert_eq!(root, tmp.path().to_string_lossy());
        assert_eq!(theme.body, "#B89875");
        assert_eq!(theme.outline, "#3D2F24");
        assert_eq!(theme.accent, "#1D1914");
        assert_eq!(theme.shirt, "#AA9370");
        assert_eq!(theme.sprite, None);
    }

    #[test]
    fn reads_existing_valid_theme_sprite() {
        let tmp = tempfile::tempdir().unwrap();
        let colors_path = tmp.path().join(PREFERRED_THEME_DIR).join("colors.json");
        write(
            &colors_path,
            r##"{
  "sprite": "JELLY",
  "palette": {
    "body": "#b89875",
    "outline": "#3d2f24",
    "accent": "#1d1914",
    "shirt": "#aa9370"
  }
}"##,
        );

        let (_root, theme) = discover_repo_theme(tmp.path().to_string_lossy().as_ref()).unwrap();

        assert_eq!(theme.sprite.as_deref(), Some("jelly"));
    }

    #[test]
    fn generates_missing_theme_without_colliding_with_siblings() {
        let tmp = tempfile::tempdir().unwrap();

        write_theme_file(
            &tmp.path().join("buildooor"),
            PREFERRED_THEME_DIR,
            "#B89875",
        );

        let repo_root = tmp.path().join("weathr");
        std::fs::create_dir_all(repo_root.join(PREFERRED_THEME_DIR)).unwrap();
        let cwd = repo_root.join("app");
        std::fs::create_dir_all(&cwd).unwrap();

        let (_root, theme) = discover_repo_theme(cwd.to_string_lossy().as_ref()).unwrap();
        let contents =
            std::fs::read_to_string(repo_root.join(PREFERRED_THEME_DIR).join("colors.json"))
                .unwrap();

        assert!(contents.contains(&theme.body));
        assert_ne!(theme.body, "#B89875");
        assert_ne!(theme.outline, "#3D2F24");
        assert_ne!(theme.accent, "#1D1914");
        assert_ne!(theme.shirt, "#AA9370");
        assert!(!contents.contains("\"sprite\""));
    }

    #[test]
    fn reads_legacy_theme_when_preferred_cache_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("buildooor");
        write_theme_file(&repo_root, LEGACY_THEME_DIR, "#B89875");

        let cwd = repo_root.join("app");
        std::fs::create_dir_all(&cwd).unwrap();

        let (root, theme) = discover_repo_theme(cwd.to_string_lossy().as_ref()).unwrap();

        assert_eq!(root, repo_root.to_string_lossy());
        assert_eq!(theme.body, "#B89875");
        assert!(!repo_root
            .join(PREFERRED_THEME_DIR)
            .join("colors.json")
            .exists());
    }

    #[test]
    fn prefers_preferred_cache_over_legacy_cache_when_both_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("recipe-cycle-app");
        write_theme_file(&repo_root, LEGACY_THEME_DIR, "#B89875");
        write_theme_file(&repo_root, PREFERRED_THEME_DIR, "#4FA66A");

        let cwd = repo_root.join("src");
        std::fs::create_dir_all(&cwd).unwrap();

        let (_root, theme) = discover_repo_theme(cwd.to_string_lossy().as_ref()).unwrap();

        assert_eq!(theme.body, "#4FA66A");
    }

    #[test]
    fn generated_theme_stays_stable_after_new_sibling_theme_appears() {
        let tmp = tempfile::tempdir().unwrap();
        write_theme_file(
            &tmp.path().join("buildooor"),
            PREFERRED_THEME_DIR,
            "#B89875",
        );

        let repo_root = tmp.path().join("weathr");
        std::fs::create_dir_all(repo_root.join(PREFERRED_THEME_DIR)).unwrap();
        let cwd = repo_root.join("app");
        std::fs::create_dir_all(&cwd).unwrap();

        let (_root, first) = discover_repo_theme(cwd.to_string_lossy().as_ref()).unwrap();
        let first_file =
            std::fs::read_to_string(repo_root.join(PREFERRED_THEME_DIR).join("colors.json"))
                .unwrap();

        write_theme_file(&tmp.path().join("skills"), PREFERRED_THEME_DIR, "#4FA66A");

        let (_root, second) = discover_repo_theme(cwd.to_string_lossy().as_ref()).unwrap();
        let second_file =
            std::fs::read_to_string(repo_root.join(PREFERRED_THEME_DIR).join("colors.json"))
                .unwrap();

        assert_eq!(second, first);
        assert_eq!(second_file, first_file);
    }

    #[test]
    fn rewrites_invalid_theme_file() {
        let tmp = tempfile::tempdir().unwrap();
        let colors_path = tmp.path().join(PREFERRED_THEME_DIR).join("colors.json");
        write(&colors_path, "{ not json ");

        let (_root, theme) = discover_repo_theme(tmp.path().to_string_lossy().as_ref()).unwrap();
        let rewritten = std::fs::read_to_string(colors_path).unwrap();

        assert!(rewritten.contains(&theme.body));
        assert!(rewritten.contains("\"generated_at\""));
    }

    #[test]
    fn rewrites_partial_theme_file() {
        let tmp = tempfile::tempdir().unwrap();
        let colors_path = tmp.path().join(PREFERRED_THEME_DIR).join("colors.json");
        write(
            &colors_path,
            r##"{
  "target": "htma",
  "palette": {
    "body": "#123456"
  }
}"##,
        );

        let (_root, theme) = discover_repo_theme(tmp.path().to_string_lossy().as_ref()).unwrap();
        let rewritten = std::fs::read_to_string(colors_path).unwrap();

        assert!(rewritten.contains(&theme.body));
        assert!(rewritten.contains(&theme.outline));
        assert!(rewritten.contains(&theme.accent));
        assert!(rewritten.contains(&theme.shirt));
        assert!(!rewritten.contains("#123456"));
    }

    #[test]
    fn returns_none_when_repo_has_no_theme_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_repo_theme(tmp.path().to_string_lossy().as_ref()).is_none());
    }
}
