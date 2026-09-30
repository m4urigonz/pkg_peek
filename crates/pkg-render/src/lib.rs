//! Drawing for PKG Peek: thumbnails (icon + badge) and the preview card.
//!
//! Everything goes through a [`Renderer`], which owns the WIC, Direct2D and
//! DirectWrite factories. COM must be initialized on the calling thread.

mod card;
mod draw;
mod locale;
mod thumb;
mod wic;

pub use card::{CardModel, DeviceCache, Hit, HitTarget, Interaction};
pub use locale::{format_date, format_size, ui_languages};
pub use thumb::{Badge, MIN_BADGE_SIZE, ThumbSource};

use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1CreateFactory, ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory};
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows_core::Result;

pub use windows::Win32::Graphics::Imaging::IWICBitmap;

pub struct Renderer {
    pub(crate) wic: IWICImagingFactory,
    pub(crate) d2d: ID2D1Factory,
    pub(crate) dwrite: IDWriteFactory,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        unsafe {
            Ok(Self {
                wic: CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?,
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            })
        }
    }

    pub fn d2d_factory(&self) -> &ID2D1Factory {
        &self.d2d
    }
}

/// UTF-16 without terminator, for DirectWrite.
pub(crate) fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
