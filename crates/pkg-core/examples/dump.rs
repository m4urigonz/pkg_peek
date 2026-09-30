//! Debug dump of PKG headers and entries:
//! `cargo run -p pkg-core --example dump -- [--brief] <pkg>...`

use std::fs::File;
use std::io::BufReader;
use std::time::Instant;

use pkg_core::{PkgInfo, Ps4Pkg};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let brief = args.iter().any(|a| a == "--brief");
    for path in args.iter().filter(|a| !a.starts_with("--")) {
        let start = Instant::now();
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                println!("{name}: open: {e}");
                continue;
            }
        };
        let mut pkg = match Ps4Pkg::open(BufReader::new(file)) {
            Ok(p) => p,
            Err(e) => {
                println!("{name}: {e}");
                continue;
            }
        };
        if !brief {
            let h = pkg.header.clone();
            println!("== {path}");
            println!(
                "   type={:#x} entries={} body={:#x}+{:#x} pfs={:#x}+{:#x} pkg_size={:#x} file_len={:#x}",
                h.pkg_type, h.entry_count, h.body_offset, h.body_size,
                h.pfs_image_offset, h.pfs_image_size, h.pkg_size, pkg.file_len()
            );
            println!("   drm={:#x} content_type={:?} flags={:#010x}", h.drm_type, h.content_type, h.content_flags);
            let names = pkg.entry_names().unwrap_or_default();
            for e in &pkg.entries {
                let name = names.iter().find(|(id, _)| *id == e.id).map(|(_, n)| n.as_str()).unwrap_or("");
                println!("   {:#06x} {:<32} off={:#010x} size={:<9} enc={}", e.id, name, e.offset, e.size, e.is_encrypted());
            }
        }
        let info = match PkgInfo::read(&mut pkg) {
            Ok(info) => info,
            Err(e) => {
                println!("{name}: info: {e}");
                continue;
            }
        };
        let icon0 = pkg.read_icon0().ok().flatten().map(|b| b.len());
        let trophy = pkg.read_trophy_icon().ok().flatten().map(|b| b.len());
        let pic = pkg.read_pic1().ok().flatten().map(|b| b.len());
        if brief {
            println!(
                "{name}\n   {} | {} | {} | {} | app {} | fw {} | {:?} | missing {:?} | icon0 {icon0:?} trophy {trophy:?} pic {pic:?} | {:?}",
                info.title,
                info.title_id,
                info.category.label(),
                info.region.map(|r| r.code()).unwrap_or("-"),
                info.app_ver.as_deref().unwrap_or("-"),
                info.firmware().unwrap_or_default(),
                info.creation_date,
                info.missing_bytes(),
                start.elapsed()
            );
        } else {
            println!("{info:#?}");
            println!("   icon0={icon0:?} trophy={trophy:?} pic1={pic:?} elapsed={:?}", start.elapsed());
        }
    }
}
