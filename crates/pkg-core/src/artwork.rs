//! Icon and background lookup with fallbacks.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use crate::info::{Category, PkgInfo};
use crate::pkg::Ps4Pkg;
use crate::siblings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSource {
    /// icon0.png of this PKG.
    Pkg,
    /// icon0.png of the base game found in the same folder.
    BaseGame,
    /// ICON0.PNG of the trophy container (usually 320x176, not square).
    Trophy,
}

#[derive(Debug, Default)]
pub struct Artwork {
    pub icon: Option<(Vec<u8>, IconSource)>,
    pub background: Option<Vec<u8>>,
}

/// Loads the icon (and optionally the background). `path` enables borrowing
/// artwork from the base game next to a patch or DLC. Read errors are treated
/// as "no artwork": callers always get something to draw.
pub fn load<R: Read + Seek>(
    pkg: &mut Ps4Pkg<R>,
    info: &PkgInfo,
    path: Option<&Path>,
    want_background: bool,
) -> Artwork {
    let mut art = Artwork {
        icon: pkg.read_icon0().ok().flatten().map(|b| (b, IconSource::Pkg)),
        background: if want_background { pkg.read_pic1().ok().flatten() } else { None },
    };

    let needs_base = art.icon.is_none() || (want_background && art.background.is_none());
    if needs_base && info.category != Category::Game {
        if let Some(mut base) = path.and_then(|p| open_base_game(p, &info.title_id)) {
            if art.icon.is_none() {
                art.icon = base.read_icon0().ok().flatten().map(|b| (b, IconSource::BaseGame));
            }
            if want_background && art.background.is_none() {
                art.background = base.read_pic1().ok().flatten();
            }
        }
    }

    if art.icon.is_none() {
        art.icon = pkg.read_trophy_icon().ok().flatten().map(|b| (b, IconSource::Trophy));
    }
    art
}

fn open_base_game(path: &Path, title_id: &str) -> Option<Ps4Pkg<BufReader<File>>> {
    let base = siblings::find_base_game(path, title_id)?;
    Ps4Pkg::open(BufReader::new(File::open(base).ok()?)).ok()
}
