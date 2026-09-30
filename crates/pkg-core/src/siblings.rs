//! Finds the base-game PKG next to a patch or DLC so its icon and background
//! can be borrowed (patches rarely ship their own icon0.png / pic1.png).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::pkg::{MAGIC, Ps4Pkg};
use crate::sfo::Sfo;

/// Directory scans are cached per directory and invalidated when the
/// directory's modification time changes (files added, removed or renamed).
static CACHE: Mutex<Option<HashMap<PathBuf, DirScan>>> = Mutex::new(None);

struct DirScan {
    modified: Option<SystemTime>,
    /// (title ID from the header content ID, path) of every PS4 PKG.
    pkgs: Vec<(String, PathBuf)>,
}

/// Title ID embedded in a content ID: `UP9000-CUSA00552_00-...` -> `CUSA00552`.
pub fn title_id_of(content_id: &str) -> Option<&str> {
    content_id.get(7..16)
}

/// Path of a base-game (`CATEGORY=gd`) PKG with the same title ID, in the
/// same directory as `pkg_path`.
pub fn find_base_game(pkg_path: &Path, title_id: &str) -> Option<PathBuf> {
    let dir = pkg_path.parent()?;
    let candidates: Vec<PathBuf> = with_scan(dir, |scan| {
        scan.pkgs
            .iter()
            .filter(|(tid, p)| tid == title_id && p != pkg_path)
            .map(|(_, p)| p.clone())
            .collect()
    })?;
    candidates.into_iter().find(|p| is_base_game(p))
}

fn with_scan<T>(dir: &Path, f: impl FnOnce(&DirScan) -> T) -> Option<T> {
    let modified = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(HashMap::new);
    let fresh = cache.get(dir).is_some_and(|s| s.modified.is_some() && s.modified == modified);
    if !fresh {
        cache.insert(dir.to_path_buf(), DirScan { modified, pkgs: scan(dir) });
    }
    cache.get(dir).map(f)
}

fn scan(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read_dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pkg")))
        .filter_map(|p| Some((header_title_id(&p)?, p)))
        .collect()
}

/// Reads only the first 0x64 bytes of the file.
fn header_title_id(path: &Path) -> Option<String> {
    let mut head = [0u8; 0x64];
    File::open(path).ok()?.read_exact(&mut head).ok()?;
    if u32::from_be_bytes(head[..4].try_into().ok()?) != MAGIC {
        return None;
    }
    let cid = crate::pkg::cstr(&head[0x40..0x64]);
    title_id_of(&cid).map(str::to_string)
}

fn is_base_game(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(mut pkg) = Ps4Pkg::open(BufReader::new(file)) else {
        return false;
    };
    matches!(pkg.read_param_sfo(), Ok(Some(b)) if Sfo::parse(&b).is_ok_and(|s| s.str("CATEGORY") == Some("gd")))
}
