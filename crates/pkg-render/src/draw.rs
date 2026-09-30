//! Small Direct2D/DirectWrite toolkit shared by the thumbnail and the card.

use std::mem::ManuallyDrop;

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{IWICBitmap, IWICBitmapSource};
use windows_core::{Interface, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{Renderer, utf16};

pub const fn rgb(hex: u32) -> D2D1_COLOR_F {
    rgba(hex, 1.0)
}

pub const fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a,
    }
}

pub fn with_alpha(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a, ..c }
}

pub fn mix(a: D2D1_COLOR_F, b: D2D1_COLOR_F, t: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

pub const fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

pub fn contains(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub const TITLE_FAMILY: windows_core::PCWSTR = w!("Segoe UI Variable Display");
pub const TEXT_FAMILY: windows_core::PCWSTR = w!("Segoe UI Variable Text");

#[derive(Clone, Copy)]
pub struct Font {
    pub family: windows_core::PCWSTR,
    pub size: f32,
    pub weight: DWRITE_FONT_WEIGHT,
}

impl Font {
    pub fn text(size: f32) -> Self {
        Self { family: TEXT_FAMILY, size, weight: DWRITE_FONT_WEIGHT_NORMAL }
    }
    pub fn semibold(size: f32) -> Self {
        Self { family: TEXT_FAMILY, size, weight: DWRITE_FONT_WEIGHT_SEMI_BOLD }
    }
    pub fn bold(size: f32) -> Self {
        Self { family: TEXT_FAMILY, size, weight: DWRITE_FONT_WEIGHT_BOLD }
    }
    pub fn title(size: f32) -> Self {
        Self { family: TITLE_FAMILY, size, weight: DWRITE_FONT_WEIGHT_SEMI_BOLD }
    }
    pub fn line_height(&self) -> f32 {
        (self.size * 1.32).round()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    /// Single line, ellipsis when too long.
    None,
    /// Wrap at word boundaries, ellipsis on the last allowed line.
    Words(u32),
    /// Wrap anywhere (IDs without spaces).
    Chars(u32),
}

pub struct Text {
    pub layout: IDWriteTextLayout,
    pub width: f32,
    pub height: f32,
}

/// A render target plus the factories, with helpers in DIPs.
pub struct Painter<'a> {
    pub r: &'a Renderer,
    pub rt: &'a ID2D1RenderTarget,
    dc: Option<ID2D1DeviceContext>,
}

impl<'a> Painter<'a> {
    pub fn new(r: &'a Renderer, rt: &'a ID2D1RenderTarget) -> Self {
        unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
        Self { r, rt, dc: rt.cast().ok() }
    }

    pub fn brush(&self, c: D2D1_COLOR_F) -> Result<ID2D1SolidColorBrush> {
        unsafe { self.rt.CreateSolidColorBrush(&c, None) }
    }

    pub fn fill(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) -> Result<()> {
        let b = self.brush(c)?;
        unsafe { self.rt.FillRectangle(&r, &b) };
        Ok(())
    }

    pub fn fill_round(&self, r: D2D_RECT_F, radius: f32, c: D2D1_COLOR_F) -> Result<()> {
        let b = self.brush(c)?;
        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillRoundedRectangle(&rr, &b) };
        Ok(())
    }

    pub fn stroke_round(&self, r: D2D_RECT_F, radius: f32, c: D2D1_COLOR_F, width: f32) -> Result<()> {
        let b = self.brush(c)?;
        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawRoundedRectangle(&rr, &b, width, None::<&ID2D1StrokeStyle>) };
        Ok(())
    }

    /// Fills `r` with a linear gradient from `from` to `to` (in DIPs).
    pub fn gradient(&self, r: D2D_RECT_F, from: (f32, f32), to: (f32, f32), stops: &[(f32, D2D1_COLOR_F)]) -> Result<()> {
        let stops: Vec<D2D1_GRADIENT_STOP> =
            stops.iter().map(|&(position, color)| D2D1_GRADIENT_STOP { position, color }).collect();
        unsafe {
            let coll = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: Vector2 { X: from.0, Y: from.1 },
                endPoint: Vector2 { X: to.0, Y: to.1 },
            };
            let brush = self.rt.CreateLinearGradientBrush(&props, None, &coll)?;
            self.rt.FillRectangle(&r, &brush);
        }
        Ok(())
    }

    pub fn text(&self, s: &str, font: Font, max_width: f32, wrap: Wrap) -> Result<Text> {
        unsafe {
            let format = self.r.dwrite.CreateTextFormat(
                font.family,
                None::<&IDWriteFontCollection>,
                font.weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font.size,
                w!("es-es"),
            )?;
            let lh = font.line_height();
            format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, lh, (lh * 0.8).round())?;
            let (mode, lines) = match wrap {
                Wrap::None => (DWRITE_WORD_WRAPPING_NO_WRAP, 1),
                Wrap::Words(n) => (DWRITE_WORD_WRAPPING_WRAP, n),
                Wrap::Chars(n) => (DWRITE_WORD_WRAPPING_CHARACTER, n),
            };
            format.SetWordWrapping(mode)?;
            let granularity = if matches!(wrap, Wrap::Words(_)) {
                DWRITE_TRIMMING_GRANULARITY_WORD
            } else {
                DWRITE_TRIMMING_GRANULARITY_CHARACTER
            };
            let sign = self.r.dwrite.CreateEllipsisTrimmingSign(&format)?;
            let trimming = DWRITE_TRIMMING { granularity, delimiter: 0, delimiterCount: 0 };
            format.SetTrimming(&trimming, &sign)?;

            let max_height = lh * lines as f32 + 0.5;
            let layout = self.r.dwrite.CreateTextLayout(&utf16(s), &format, max_width.max(1.0), max_height)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            let visible_lines = m.lineCount.min(lines).max(1);
            Ok(Text {
                width: m.widthIncludingTrailingWhitespace.min(max_width),
                height: lh * visible_lines as f32,
                layout,
            })
        }
    }

    pub fn draw_text(&self, t: &Text, x: f32, y: f32, c: D2D1_COLOR_F) -> Result<()> {
        let b = self.brush(c)?;
        unsafe {
            self.rt.DrawTextLayout(Vector2 { X: x, Y: y }, &t.layout, &b, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
        Ok(())
    }

    pub fn bitmap(&self, src: &IWICBitmap) -> Result<ID2D1Bitmap> {
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let src: IWICBitmapSource = src.cast()?;
        unsafe { self.rt.CreateBitmapFromWicBitmap(&src, Some(&props)) }
    }

    /// Draws `src` (the part of the bitmap, in pixels) into `dest`, with high
    /// quality filtering when available and optional rounded corners.
    pub fn image(&self, bmp: &ID2D1Bitmap, dest: D2D_RECT_F, src: Option<D2D_RECT_F>, opacity: f32, radius: f32) -> Result<()> {
        unsafe {
            let mask = if radius > 0.0 {
                let geom = self.r.d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                    rect: dest,
                    radiusX: radius,
                    radiusY: radius,
                })?;
                Some(geom.cast::<ID2D1Geometry>()?)
            } else {
                None
            };
            if let Some(mask) = &mask {
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: dest,
                    geometricMask: ManuallyDrop::new(Some(mask.clone())),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, None::<&ID2D1Layer>);
                drop(ManuallyDrop::into_inner(params.geometricMask));
            }
            let src_ptr = src.as_ref().map(|s| s as *const D2D_RECT_F);
            match &self.dc {
                Some(dc) => dc.DrawBitmap(
                    bmp,
                    Some(&dest),
                    opacity,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    src_ptr,
                    None,
                ),
                None => self.rt.DrawBitmap(bmp, Some(&dest), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, src_ptr),
            }
            if mask.is_some() {
                self.rt.PopLayer();
            }
        }
        Ok(())
    }

    /// Draws `bmp` scaled to cover `area` (cropping the overflow).
    pub fn image_cover(&self, bmp: &ID2D1Bitmap, area: D2D_RECT_F, opacity: f32) -> Result<()> {
        let size = unsafe { bmp.GetPixelSize() };
        let (iw, ih) = (size.width as f32, size.height as f32);
        let (aw, ah) = (area.right - area.left, area.bottom - area.top);
        if iw < 1.0 || ih < 1.0 || aw < 1.0 || ah < 1.0 {
            return Ok(());
        }
        let scale = (aw / iw).max(ah / ih);
        let (sw, sh) = (aw / scale, ah / scale);
        // Keep the top of the image (PS4 backgrounds put the art up there).
        let sx = (iw - sw) / 2.0;
        let sy = ((ih - sh) * 0.3).max(0.0);
        self.image(bmp, area, Some(rect(sx, sy, sx + sw, sy + sh)), opacity, 0.0)
    }
}

/// Creates a Direct2D target drawing into a PBGRA WIC bitmap.
pub fn wic_target(r: &Renderer, target: &IWICBitmap, dpi: f32) -> Result<ID2D1RenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: dpi,
        dpiY: dpi,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    unsafe { r.d2d.CreateWicBitmapRenderTarget(target, &props) }
}
