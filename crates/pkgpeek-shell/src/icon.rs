//! Per-file icon handler, so the game icon also shows in the Details, List
//! and Small icons views (which do not use thumbnails).
//!
//! Registered on the `PkgPeek.Package` ProgID with `DefaultIcon = %1`. It is
//! loaded inside explorer.exe, so extraction is deferred to Explorer's
//! background threads with `E_PENDING`.

use std::cell::RefCell;
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use pkg_render::Renderer;
use windows::Win32::Foundation::{E_NOTIMPL, E_POINTER, E_UNEXPECTED, S_FALSE};
use windows::Win32::System::Com::{IPersist_Impl, IPersistFile, IPersistFile_Impl, STGM};
use windows::Win32::UI::Shell::{GIL_ASYNC, GIL_NOTFILENAME, GIL_PERINSTANCE, IExtractIconW, IExtractIconW_Impl};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows_core::{BOOL, GUID, HRESULT, Interface, PCWSTR, PWSTR, Result, implement};

use crate::{ModuleRef, dll_path, guard, guids, path_from};

/// "Extract later, on a background thread" (declared in Urlmon, not worth the feature).
const E_PENDING: HRESULT = HRESULT(0x8000_000A_u32 as i32);

#[implement(IExtractIconW, IPersistFile)]
pub struct IconHandler {
    path: RefCell<Option<PathBuf>>,
    _module: ModuleRef,
}

impl IconHandler {
    pub fn new() -> Self {
        Self { path: RefCell::new(None), _module: ModuleRef::new() }
    }

    fn path(&self) -> Result<PathBuf> {
        self.path.borrow().clone().ok_or_else(|| E_UNEXPECTED.into())
    }
}

fn is_ps4_pkg(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && pkg_core::is_ps4_pkg(&magic)
}

/// Icon index for the shell's icon cache: changes when the file does.
fn cache_index(path: &Path) -> i32 {
    let mut h = DefaultHasher::new();
    if let Ok(meta) = std::fs::metadata(path) {
        meta.len().hash(&mut h);
        meta.modified().ok().hash(&mut h);
    }
    (h.finish() & 0x7FFF_FFFF) as i32
}

/// Copies `s` into the shell's buffer, NUL-terminated.
fn write_wide(s: &str, buf: PWSTR, cch: u32) -> Result<()> {
    let wide: Vec<u16> = s.encode_utf16().collect();
    if buf.is_null() || wide.len() + 1 > cch as usize {
        return Err(E_POINTER.into());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr(), buf.0, wide.len());
        *buf.0.add(wide.len()) = 0;
    }
    Ok(())
}

impl IExtractIconW_Impl for IconHandler_Impl {
    fn GetIconLocation(&self, flags: u32, file: PWSTR, cch: u32, index: *mut i32, out_flags: *mut u32) -> Result<()> {
        guard(|| {
            if index.is_null() || out_flags.is_null() {
                return Err(E_POINTER.into());
            }
            if flags & GIL_ASYNC != 0 {
                return Err(E_PENDING.into());
            }
            let path = self.path()?;
            unsafe {
                if is_ps4_pkg(&path) {
                    write_wide(&path.to_string_lossy(), file, cch)?;
                    *index = cache_index(&path);
                    *out_flags = GIL_NOTFILENAME | GIL_PERINSTANCE;
                } else {
                    // Generic PKG icon: resource index 0 of this DLL.
                    write_wide(&dll_path()?.to_string_lossy(), file, cch)?;
                    *index = 0;
                    *out_flags = 0;
                }
            }
            Ok(())
        })
    }

    fn Extract(&self, _file: &PCWSTR, _index: u32, large: *mut HICON, small: *mut HICON, sizes: u32) -> Result<()> {
        guard(|| {
            let path = self.path()?;
            let renderer = Renderer::new()?;
            let file = std::fs::File::open(&path).map_err(crate::io_error)?;
            let Some(source) = renderer.thumb_source(BufReader::new(file), Some(&path)) else {
                // Let the shell extract from the location we returned.
                return Err(windows_core::Error::from_hresult(S_FALSE));
            };
            let make = |size: u32| -> Result<HICON> {
                let bitmap = renderer.thumbnail(source.icon.as_ref(), source.badge.as_ref(), size.clamp(16, 256))?;
                renderer.to_hicon(&bitmap.cast()?)
            };
            unsafe {
                if !large.is_null() {
                    *large = make(sizes & 0xFFFF)?;
                }
                if !small.is_null() {
                    *small = make((sizes >> 16) & 0xFFFF)?;
                }
            }
            Ok(())
        })
    }
}

impl IPersist_Impl for IconHandler_Impl {
    fn GetClassID(&self) -> Result<GUID> {
        Ok(guids::CLSID_ICON)
    }
}

impl IPersistFile_Impl for IconHandler_Impl {
    fn IsDirty(&self) -> HRESULT {
        S_FALSE
    }

    fn Load(&self, path: &PCWSTR, _mode: STGM) -> Result<()> {
        guard(|| {
            *self.path.borrow_mut() = Some(path_from(path)?);
            Ok(())
        })
    }

    fn Save(&self, _path: &PCWSTR, _remember: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SaveCompleted(&self, _path: &PCWSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetCurFile(&self) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
}
