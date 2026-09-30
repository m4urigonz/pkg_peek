//! IThumbnailProvider: the game icon with a type badge (medium/large icons,
//! tiles, details pane).
//!
//! Initialized with a file path rather than a stream so that patches can
//! borrow the icon of the base game in the same folder. This requires the
//! handler to be registered with `DisableProcessIsolation`.

use std::cell::RefCell;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use pkg_render::Renderer;
use windows::Win32::Foundation::{E_FAIL, E_POINTER, E_UNEXPECTED};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithFile_Impl};
use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_ARGB};
use windows_core::{Interface, PCWSTR, Result, implement};

use crate::{ModuleRef, already_initialized, guard, io_error, path_from};

#[implement(IThumbnailProvider, IInitializeWithFile, Agile = false)]
pub struct ThumbnailProvider {
    path: RefCell<Option<PathBuf>>,
    _module: ModuleRef,
}

impl ThumbnailProvider {
    pub fn new() -> Self {
        Self { path: RefCell::new(None), _module: ModuleRef::new() }
    }
}

impl IInitializeWithFile_Impl for ThumbnailProvider_Impl {
    fn Initialize(&self, path: &PCWSTR, _mode: u32) -> Result<()> {
        guard(|| {
            let mut slot = self.path.borrow_mut();
            if slot.is_some() {
                return Err(already_initialized());
            }
            *slot = Some(path_from(path)?);
            Ok(())
        })
    }
}

impl IThumbnailProvider_Impl for ThumbnailProvider_Impl {
    fn GetThumbnail(&self, cx: u32, phbmp: *mut HBITMAP, alpha: *mut WTS_ALPHATYPE) -> Result<()> {
        guard(|| {
            if phbmp.is_null() || alpha.is_null() {
                return Err(E_POINTER.into());
            }
            let path = self.path.borrow().clone().ok_or(E_UNEXPECTED)?;
            let file = File::open(&path).map_err(io_error)?;
            let renderer = Renderer::new()?;
            // Not a PS4 PKG: fail so Explorer falls back to the file type icon.
            let source = renderer.thumb_source(BufReader::new(file), Some(&path)).ok_or(E_FAIL)?;
            let bitmap = renderer.thumbnail(source.icon.as_ref(), source.badge.as_ref(), cx.clamp(16, 2560))?;
            let hbmp = renderer.to_hbitmap(&bitmap.cast()?)?;
            unsafe {
                *phbmp = hbmp;
                *alpha = WTSAT_ARGB;
            }
            Ok(())
        })
    }
}
