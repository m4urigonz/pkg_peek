//! Command-line tools, mainly for testing the rendering without Explorer.
//!
//! ```text
//! pkgpeek-cli info  <pkg>... [--json]
//! pkgpeek-cli thumb <pkg> <out.png> [--size N]
//! pkgpeek-cli card  <pkg> <out.png> [--width W] [--height H] [--scale S]
//! pkgpeek-cli ico   <out.ico>          (generic PKG icon used by the shell DLL)
//! pkgpeek-cli shelltest <dll> <pkg> <outdir>   (drives the COM handlers directly)
//! ```

mod shelltest;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::ExitCode;

use pkg_core::{PkgInfo, Ps4Pkg};
use pkg_render::{CardModel, Renderer, format_size, ui_languages};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") => info(&args[1..]),
        Some("thumb") => thumb(&args[1..]),
        Some("card") => card(&args[1..]),
        Some("ico") => ico(&args[1..]),
        Some("capture") => match &args[1..] {
            [hwnd, out] => renderer().and_then(|r| Ok(shelltest::capture_hwnd(&r, hwnd.parse()?, out)?)),
            _ => Err("uso: capture <hwnd> <out.png>".into()),
        },
        Some("shellcheck") => match &args[1..] {
            [pkg, out] => renderer().and_then(|r| Ok(shelltest::check(&r, pkg, Path::new(out))?)),
            _ => Err("uso: shellcheck <pkg> <outdir>".into()),
        },
        Some("shelltest") => match &args[1..] {
            [dll, pkg, out] => renderer().and_then(|r| Ok(shelltest::run(&r, dll, pkg, Path::new(out))?)),
            _ => Err("uso: shelltest <dll> <pkg> <outdir>".into()),
        },
        _ => Err("uso: pkgpeek-cli info|thumb|card|ico ... (ver cli.rs)".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

type CliResult = Result<(), Box<dyn std::error::Error>>;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn positional(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--json" {
        } else if a.starts_with("--") {
            skip = true;
        } else {
            out.push(a);
        }
    }
    out
}

fn renderer() -> Result<Renderer, Box<dyn std::error::Error>> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    Ok(Renderer::new()?)
}

fn info(args: &[String]) -> CliResult {
    let json = args.iter().any(|a| a == "--json");
    let languages = ui_languages();
    for path in positional(args) {
        let mut pkg = Ps4Pkg::open(BufReader::new(File::open(path)?)).map_err(|e| format!("{path}: {e}"))?;
        let info = PkgInfo::read(&mut pkg).map_err(|e| format!("{path}: {e}"))?;
        let rows: Vec<(&str, String)> = vec![
            ("file", path.clone()),
            ("title", info.title_for(&languages).to_string()),
            ("title_id", info.title_id.clone()),
            ("content_id", info.content_id.clone()),
            ("category", info.category.label().to_string()),
            ("app_ver", info.app_ver.clone().unwrap_or_default()),
            ("version", info.version.clone().unwrap_or_default()),
            ("firmware", info.firmware().unwrap_or_default()),
            ("region", info.region.map(|r| r.label().to_string()).unwrap_or_default()),
            ("created", info.creation_date.clone().unwrap_or_default()),
            ("sdk", info.sdk_ver.clone().unwrap_or_default()),
            ("size", format_size(info.file_len)),
            ("missing", info.missing_bytes().map(format_size).unwrap_or_default()),
            ("notes", info.change_notes.clone().unwrap_or_default()),
        ];
        if json {
            let body: Vec<String> = rows.iter().map(|(k, v)| format!("  \"{k}\": \"{}\"", escape(v))).collect();
            println!("{{\n{}\n}}", body.join(",\n"));
        } else {
            for (k, v) in rows.iter().filter(|(_, v)| !v.is_empty()) {
                println!("{k:>10}: {}", v.replace('\n', "\n            "));
            }
            println!();
        }
    }
    Ok(())
}

fn escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}

fn thumb(args: &[String]) -> CliResult {
    let pos = positional(args);
    let [pkg, out] = pos[..] else { return Err("uso: thumb <pkg> <out.png> [--size N]".into()) };
    let size: u32 = flag(args, "--size").map(|s| s.parse()).transpose()?.unwrap_or(256);
    let r = renderer()?;
    let source = r
        .thumb_source(BufReader::new(File::open(pkg)?), Some(Path::new(pkg)))
        .ok_or("No es un PKG de PS4")?;
    let bmp = r.thumbnail(source.icon.as_ref(), source.badge.as_ref(), size)?;
    r.save_png(&windows_core::Interface::cast(&bmp)?, out)?;
    println!("{out} ({size}x{size}, badge {:?})", source.badge);
    Ok(())
}

fn card(args: &[String]) -> CliResult {
    let pos = positional(args);
    let [pkg, out] = pos[..] else { return Err("uso: card <pkg> <out.png> [--width W] [--height H]".into()) };
    let width: u32 = flag(args, "--width").map(|s| s.parse()).transpose()?.unwrap_or(420);
    let height: u32 = flag(args, "--height").map(|s| s.parse()).transpose()?.unwrap_or(720);
    let scale: f32 = flag(args, "--scale").map(|s| s.parse()).transpose()?.unwrap_or(1.0);
    let r = renderer()?;
    let file = File::open(pkg)?;
    let len = file.metadata()?.len();
    let name = Path::new(pkg).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let model = CardModel::load(&r, BufReader::new(file), Some(Path::new(pkg)), name, len, &ui_languages());
    let bmp = r.render_card(&model, width, height, scale)?;
    r.save_png(&windows_core::Interface::cast(&bmp)?, out)?;
    println!("{out} ({width}x{height} @{scale})");
    Ok(())
}

/// Writes a multi-size .ico (PNG entries) with the generic PKG glyph.
fn ico(args: &[String]) -> CliResult {
    let pos = positional(args);
    let [out] = pos[..] else { return Err("uso: ico <out.ico>".into()) };
    let r = renderer()?;
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256];
    let mut images = Vec::new();
    let tmp = std::env::temp_dir().join("pkgpeek-ico.png");
    let tmp_str = tmp.to_string_lossy().into_owned();
    for &size in &sizes {
        let bmp = r.thumbnail(None, None, size)?;
        r.save_png(&windows_core::Interface::cast(&bmp)?, &tmp_str)?;
        images.push(std::fs::read(&tmp)?);
    }
    let _ = std::fs::remove_file(&tmp);

    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, png) in sizes.iter().zip(&images) {
        let dim = if size >= 256 { 0 } else { size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &images {
        ico.extend_from_slice(png);
    }
    std::fs::write(out, ico)?;
    println!("{out}");
    Ok(())
}
