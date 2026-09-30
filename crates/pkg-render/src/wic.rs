//! WIC helpers: decode, scale, convert to GDI objects, save as PNG.

use std::ptr::null;

use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
    HBITMAP,
};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows_core::{HSTRING, Interface, PCWSTR, Result};

use crate::Renderer;

impl Renderer {
    /// Decodes an image (PNG, JPEG, ...) into a premultiplied BGRA bitmap
    /// held in memory, so `bytes` can be dropped afterwards.
    pub fn decode(&self, bytes: &[u8]) -> Result<IWICBitmap> {
        unsafe {
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let decoder = self.wic.CreateDecoderFromStream(&stream, null(), WICDecodeMetadataCacheOnDemand)?;
            let frame = decoder.GetFrame(0)?;
            let converted = self.convert(&frame, &GUID_WICPixelFormat32bppPBGRA)?;
            self.wic.CreateBitmapFromSource(&converted, WICBitmapCacheOnLoad)
        }
    }

    pub(crate) fn convert(&self, src: &IWICBitmapSource, format: &windows_core::GUID) -> Result<IWICBitmapSource> {
        unsafe {
            let conv = self.wic.CreateFormatConverter()?;
            conv.Initialize(
                src,
                format,
                WICBitmapDitherTypeNone,
                None::<&IWICPalette>,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
            conv.cast()
        }
    }

    /// High-quality resize.
    pub fn scale(&self, src: &IWICBitmapSource, width: u32, height: u32) -> Result<IWICBitmap> {
        unsafe {
            let scaler = self.wic.CreateBitmapScaler()?;
            scaler.Initialize(src, width.max(1), height.max(1), WICBitmapInterpolationModeHighQualityCubic)?;
            self.wic.CreateBitmapFromSource(&scaler, WICBitmapCacheOnLoad)
        }
    }

    pub fn size_of(src: &IWICBitmapSource) -> (u32, u32) {
        let (mut w, mut h) = (0, 0);
        unsafe {
            let _ = src.GetSize(&mut w, &mut h);
        }
        (w, h)
    }

    /// Average color of an image, used to tint cards without a background.
    pub fn average_color(&self, src: &IWICBitmapSource) -> Option<D2D1_COLOR_F> {
        unsafe {
            let scaler = self.wic.CreateBitmapScaler().ok()?;
            scaler.Initialize(src, 1, 1, WICBitmapInterpolationModeFant).ok()?;
            let mut px = [0u8; 4];
            scaler.CopyPixels(null(), 4, &mut px).ok()?;
            let a = px[3] as f32 / 255.0;
            if a < 0.05 {
                return None;
            }
            // Premultiplied BGRA -> straight RGB.
            Some(D2D1_COLOR_F {
                r: px[2] as f32 / 255.0 / a,
                g: px[1] as f32 / 255.0 / a,
                b: px[0] as f32 / 255.0 / a,
                a: 1.0,
            })
        }
    }

    /// Top-down 32 bpp DIB with straight alpha, as IThumbnailProvider and
    /// icons expect.
    pub fn to_hbitmap(&self, src: &IWICBitmapSource) -> Result<HBITMAP> {
        let (w, h) = Self::size_of(src);
        let straight = self.convert(src, &GUID_WICPixelFormat32bppBGRA)?;
        unsafe {
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let hbmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let len = (w * h * 4) as usize;
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, len);
            if let Err(e) = straight.CopyPixels(null(), w * 4, pixels) {
                let _ = DeleteObject(hbmp.into());
                return Err(e);
            }
            Ok(hbmp)
        }
    }

    pub fn to_hicon(&self, src: &IWICBitmapSource) -> Result<HICON> {
        let (w, h) = Self::size_of(src);
        let color = self.to_hbitmap(src)?;
        unsafe {
            let mask = CreateBitmap(w as i32, h as i32, 1, 1, None);
            let info = ICONINFO { fIcon: true.into(), hbmColor: color, hbmMask: mask, ..Default::default() };
            let icon = CreateIconIndirect(&info);
            let _ = DeleteObject(color.into());
            let _ = DeleteObject(mask.into());
            icon
        }
    }

    pub fn save_png(&self, src: &IWICBitmapSource, path: &str) -> Result<()> {
        let (w, h) = Self::size_of(src);
        let straight = self.convert(src, &GUID_WICPixelFormat32bppBGRA)?;
        unsafe {
            let stream = self.wic.CreateStream()?;
            let path = HSTRING::from(path);
            stream.InitializeFromFilename(PCWSTR(path.as_ptr()), GENERIC_WRITE.0)?;
            let encoder = self.wic.CreateEncoder(&GUID_ContainerFormatPng, null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            let mut options = None;
            encoder.CreateNewFrame(&mut frame, &mut options)?;
            let frame = frame.ok_or_else(windows_core::Error::empty)?;
            frame.Initialize(options.as_ref())?;
            frame.SetSize(w, h)?;
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(&straight, null())?;
            frame.Commit()?;
            encoder.Commit()
        }
    }
}

/// Debug helpers used by `pkgpeek-cli shelltest` to inspect handler output.
impl Renderer {
    pub fn save_hbitmap_png(&self, hbmp: HBITMAP, path: &str) -> Result<()> {
        let bmp = unsafe {
            self.wic.CreateBitmapFromHBITMAP(hbmp, windows::Win32::Graphics::Gdi::HPALETTE::default(), WICBitmapUseAlpha)?
        };
        self.save_png(&bmp.cast()?, path)
    }

    pub fn save_hicon_png(&self, icon: HICON, path: &str) -> Result<()> {
        let bmp = unsafe { self.wic.CreateBitmapFromHICON(icon)? };
        self.save_png(&bmp.cast()?, path)
    }
}
