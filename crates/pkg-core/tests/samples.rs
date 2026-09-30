//! Parses every PKG in `$PKGPEEK_SAMPLES` (skipped when unset):
//! `PKGPEEK_SAMPLES=D:\Descargas cargo test -p pkg-core --test samples -- --nocapture`

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pkg_core::{Category, PkgInfo, Ps4Pkg, artwork};

fn sample_files() -> Vec<PathBuf> {
    let Ok(dir) = std::env::var("PKGPEEK_SAMPLES") else {
        return Vec::new();
    };
    std::fs::read_dir(dir)
        .expect("PKGPEEK_SAMPLES must be a directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pkg")))
        .collect()
}

#[test]
fn every_ps4_sample_parses_quickly() {
    for path in sample_files() {
        let mut magic = [0u8; 4];
        File::open(&path).unwrap().read_exact(&mut magic).unwrap();
        let file = BufReader::new(File::open(&path).unwrap());
        if !pkg_core::is_ps4_pkg(&magic) {
            assert!(Ps4Pkg::open(file).is_err(), "{path:?} should be rejected");
            continue;
        }
        let start = Instant::now();
        let mut pkg = Ps4Pkg::open(file).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let info = PkgInfo::read(&mut pkg).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let art = artwork::load(&mut pkg, &info, Some(&path), true);
        let elapsed = start.elapsed();
        println!(
            "{:<50} {:<10} {:<14} icon={:?} bg={}  {elapsed:?}",
            path.file_name().unwrap().to_string_lossy(),
            info.title_id,
            info.category.label(),
            art.icon.as_ref().map(|(_, s)| *s),
            art.background.is_some(),
        );
        assert!(!info.title_id.is_empty(), "{path:?}: no title ID");
        assert!(art.icon.is_some(), "{path:?}: no icon");
        if info.category == Category::Patch {
            assert!(info.app_ver.is_some(), "{path:?}: patch without APP_VER");
        }
        assert!(elapsed < Duration::from_secs(2), "{path:?}: too slow ({elapsed:?})");
    }
}
