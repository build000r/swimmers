use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::SystemTime;

use tokio::sync::RwLock;

use crate::types::SpritePack;

// ---------------------------------------------------------------------------
// Cache types
// ---------------------------------------------------------------------------

struct CacheEntry {
    pack: SpritePack,
    /// mtime of the `.throngterm/sprites/` directory at the time this entry
    /// was populated.  Used to detect in-place file edits.
    dir_mtime: SystemTime,
}

static SPRITE_CACHE: OnceLock<RwLock<HashMap<String, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static RwLock<HashMap<String, CacheEntry>> {
    SPRITE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Walk up from `cwd` looking for a `.throngterm/sprites/` directory.
///
/// Returns `(project_root_path, SpritePack)` when all four SVG files
/// (`active.svg`, `drowsy.svg`, `sleeping.svg`, `deep_sleep.svg`) are
/// present and readable.  Returns `None` on any error, logging a `warn!`
/// for diagnosable failures.
pub async fn discover_sprite_pack(cwd: &str) -> Option<(String, SpritePack)> {
    let sprites_dir = walk_to_sprites_dir(cwd)?;
    let project_root = Path::new(&sprites_dir)
        .parent()   // .throngterm/
        .and_then(|p| p.parent())  // project root
        .map(|p| p.to_string_lossy().into_owned())?;

    // Check the current mtime of the sprites directory so we can invalidate
    // stale cache entries when files are updated in-place.
    let current_mtime = dir_mtime(&sprites_dir);

    // Fast path: check the cache under a read lock.
    {
        let cache_guard = cache().read().await;
        if let Some(entry) = cache_guard.get(&sprites_dir) {
            if let Some(mtime) = current_mtime {
                if entry.dir_mtime == mtime {
                    return Some((project_root, entry.pack.clone()));
                }
            }
        }
    }

    // Cache miss or stale entry — read the SVG files from disk.
    let pack = read_sprite_pack(&sprites_dir)?;

    // Populate the cache under a write lock.
    {
        let mut cache_guard = cache().write().await;
        cache_guard.insert(
            sprites_dir.clone(),
            CacheEntry {
                pack: pack.clone(),
                dir_mtime: current_mtime.unwrap_or(SystemTime::UNIX_EPOCH),
            },
        );
    }

    Some((project_root, pack))
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Walk up from `cwd` (inclusive) to `/`, returning the first
/// `<dir>/.throngterm/sprites/` path that exists as a directory.
fn walk_to_sprites_dir(cwd: &str) -> Option<String> {
    let start = Path::new(cwd);
    let mut current = start;

    loop {
        let candidate = current.join(".throngterm").join("sprites");
        if candidate.is_dir() {
            return Some(candidate.to_string_lossy().into_owned());
        }

        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return None,
        }
    }
}

/// Read all four SVG files from `sprites_dir`.  Returns `None` (and logs a
/// warning) if any file is missing or unreadable.
fn read_sprite_pack(sprites_dir: &str) -> Option<SpritePack> {
    let dir = Path::new(sprites_dir);

    let active = read_svg(dir, "active.svg", sprites_dir)?;
    let drowsy = read_svg(dir, "drowsy.svg", sprites_dir)?;
    let sleeping = read_svg(dir, "sleeping.svg", sprites_dir)?;
    let deep_sleep = read_svg(dir, "deep_sleep.svg", sprites_dir)?;

    Some(SpritePack {
        active,
        drowsy,
        sleeping,
        deep_sleep,
    })
}

/// Read a single SVG file, returning `None` and emitting a `tracing::warn!`
/// if the file is missing or cannot be read.
fn read_svg(dir: &Path, filename: &str, sprites_dir: &str) -> Option<String> {
    let path = dir.join(filename);
    match std::fs::read_to_string(&path) {
        Ok(contents) => Some(contents),
        Err(err) => {
            tracing::warn!(
                sprites_dir = %sprites_dir,
                file = %filename,
                "failed to read sprite SVG file: {}",
                err
            );
            None
        }
    }
}

/// Return the mtime of a directory, or `None` if it cannot be stat'd.
fn dir_mtime(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &std::path::Path, name: &str, content: &str) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    /// Build a temp dir with the full .throngterm/sprites/ convention layout
    /// and verify that discover_sprite_pack returns a valid pack with the
    /// project root (the parent of .throngterm/).
    #[tokio::test]
    async fn discovers_pack_from_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let sprites_dir = tmp.path().join(".throngterm").join("sprites");
        std::fs::create_dir_all(&sprites_dir).unwrap();

        write_file(&sprites_dir, "active.svg", "<svg id='active'/>");
        write_file(&sprites_dir, "drowsy.svg", "<svg id='drowsy'/>");
        write_file(&sprites_dir, "sleeping.svg", "<svg id='sleeping'/>");
        write_file(&sprites_dir, "deep_sleep.svg", "<svg id='deep_sleep'/>");

        let cwd = tmp.path().to_string_lossy().into_owned();
        let result = discover_sprite_pack(&cwd).await;

        assert!(result.is_some(), "expected sprite pack to be discovered");
        let (root, pack) = result.unwrap();
        assert_eq!(root, cwd);
        assert!(pack.active.contains("active"));
        assert!(pack.drowsy.contains("drowsy"));
        assert!(pack.sleeping.contains("sleeping"));
        assert!(pack.deep_sleep.contains("deep_sleep"));
    }

    /// Walk-up: cwd is a sub-directory of the repo root; sprites live in the
    /// repo root.
    #[tokio::test]
    async fn walks_up_to_find_sprites() {
        let tmp = tempfile::tempdir().unwrap();
        let sprites_dir = tmp.path().join(".throngterm").join("sprites");
        std::fs::create_dir_all(&sprites_dir).unwrap();

        write_file(&sprites_dir, "active.svg", "<svg/>");
        write_file(&sprites_dir, "drowsy.svg", "<svg/>");
        write_file(&sprites_dir, "sleeping.svg", "<svg/>");
        write_file(&sprites_dir, "deep_sleep.svg", "<svg/>");

        let sub = tmp.path().join("src").join("lib");
        std::fs::create_dir_all(&sub).unwrap();

        let cwd = sub.to_string_lossy().into_owned();
        let result = discover_sprite_pack(&cwd).await;

        assert!(result.is_some(), "expected walk-up to find sprites");
        let (root, _) = result.unwrap();
        assert_eq!(root, tmp.path().to_string_lossy().as_ref());
    }

    /// Missing a single SVG file → returns None and does not cache a partial
    /// pack.
    #[tokio::test]
    async fn rejects_partial_pack() {
        let tmp = tempfile::tempdir().unwrap();
        let sprites_dir = tmp.path().join(".throngterm").join("sprites");
        std::fs::create_dir_all(&sprites_dir).unwrap();

        write_file(&sprites_dir, "active.svg", "<svg/>");
        write_file(&sprites_dir, "drowsy.svg", "<svg/>");
        // sleeping.svg intentionally omitted
        write_file(&sprites_dir, "deep_sleep.svg", "<svg/>");

        let cwd = tmp.path().to_string_lossy().into_owned();
        let result = discover_sprite_pack(&cwd).await;
        assert!(result.is_none(), "partial pack should be rejected");
    }

    /// No .throngterm/ anywhere in the tree → returns None.
    #[tokio::test]
    async fn returns_none_when_no_convention_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_string_lossy().into_owned();
        let result = discover_sprite_pack(&cwd).await;
        assert!(result.is_none());
    }
}
