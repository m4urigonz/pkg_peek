//! Explorer thumbnails and icons: the game icon plus a small type badge.

use pkg_core::{Category, PkgInfo};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Imaging::{GUID_WICPixelFormat32bppPBGRA, IWICBitmap, WICBitmapCacheOnLoad};
use windows_core::{Interface, Result};

use crate::Renderer;
use crate::draw::{Font, Painter, Wrap, rect, rgb, rgba, wic_target};

/// Marks what kind of PKG a thumbnail is, so a base game, its update and its
/// DLC (which share the same icon) can be told apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Badge {
    Update(Option<String>),
    Dlc,
    Incomplete,
}

impl Badge {
    pub fn for_info(info: &PkgInfo) -> Option<Self> {
        if info.missing_bytes().is_some() {
            return Some(Self::Incomplete);
        }
        match info.category {
            Category::Patch => Some(Self::Update(info.short_app_ver())),
            Category::Dlc => Some(Self::Dlc),
            _ => None,
        }
    }

    fn text(&self, compact: bool) -> String {
        match self {
            Self::Update(Some(v)) if !compact => format!("UPD {v}"),
            Self::Update(_) => "UPD".into(),
            Self::Dlc => "DLC".into(),
            Self::Incomplete => "!".into(),
        }
    }

    fn colors(&self) -> (D2D1_COLOR_F, D2D1_COLOR_F) {
        match self {
            Self::Update(_) => (rgb(0xF5A623), rgb(0x231500)),
            Self::Dlc => (rgb(0x8E5CF7), rgb(0xFFFFFF)),
            Self::Incomplete => (rgb(0xE5484D), rgb(0xFFFFFF)),
        }
    }
}

/// Badges are drawn from this size up; below it they would be unreadable.
pub const MIN_BADGE_SIZE: u32 = 64;

impl Renderer {
    /// Square `size`x`size` thumbnail. Without `icon` a generic PKG glyph is
    /// drawn instead.
    pub fn thumbnail(&self, icon: Option<&IWICBitmap>, badge: Option<&Badge>, size: u32) -> Result<IWICBitmap> {
        let target = unsafe { self.wic.CreateBitmap(size, size, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)? };
        let rt = wic_target(self, &target, 96.0)?;
        let p = Painter::new(self, &rt);
        let s = size as f32;
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&rgba(0, 0.0)));
        }
        match icon {
            Some(icon) => {
                let src = icon.cast()?;
                let (iw, ih) = Self::size_of(&src);
                let fit = (s / iw.max(1) as f32).min(s / ih.max(1) as f32);
                let (dw, dh) = ((iw as f32 * fit).round().max(1.0), (ih as f32 * fit).round().max(1.0));
                // Pre-scale with WIC: sharper than letting Direct2D minify.
                let scaled = self.scale(&src, dw as u32, dh as u32)?;
                let bmp = p.bitmap(&scaled)?;
                let (x, y) = (((s - dw) / 2.0).floor(), ((s - dh) / 2.0).floor());
                let radius = if size >= 48 && iw == ih { (s * 0.06).round() } else { 0.0 };
                p.image(&bmp, rect(x, y, x + dw, y + dh), None, 1.0, radius)?;
            }
            None => draw_placeholder(&p, s)?,
        }
        if let Some(badge) = badge.filter(|_| size >= MIN_BADGE_SIZE) {
            draw_badge(&p, badge, s)?;
        }
        unsafe { rt.EndDraw(None, None)? };
        Ok(target)
    }
}

/// Generic "PKG" tile in PlayStation blue.
pub(crate) fn draw_placeholder(p: &Painter, s: f32) -> Result<()> {
    let inset = (s * 0.06).round();
    let r = rect(inset, inset, s - inset, s - inset);
    let radius = (s * 0.12).round();
    p.fill_round(r, radius, rgb(0x0070D1))?;
    let label = p.text("PKG", Font::bold((s * 0.26).max(6.0)), s, Wrap::None)?;
    unsafe {
        label.layout.SetMaxWidth(r.right - r.left)?;
        label.layout.SetMaxHeight(r.bottom - r.top)?;
        label.layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        label.layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    }
    p.draw_text(&label, r.left, r.top, rgb(0xFFFFFF))
}

fn draw_badge(p: &Painter, badge: &Badge, s: f32) -> Result<()> {
    let h = (s * 0.165).round().max(13.0);
    let margin = (s * 0.03).round();
    let (bg, fg) = badge.colors();
    let font = Font::bold((h * 0.62).round());
    // Drop the version if the full label would cover most of the icon.
    let mut text = p.text(&badge.text(false), font, s, Wrap::None)?;
    if text.width + h * 0.7 > s * 0.8 {
        text = p.text(&badge.text(true), font, s, Wrap::None)?;
    }
    let w = (text.width + h * 0.7).round().max(h);
    let r = rect(s - margin - w, s - margin - h, s - margin, s - margin);
    // Dark halo keeps the badge readable over light icons.
    let halo = (s / 128.0).max(1.0);
    p.fill_round(
        rect(r.left - halo, r.top - halo, r.right + halo, r.bottom + halo),
        h / 2.0 + halo,
        rgba(0x000000, 0.45),
    )?;
    p.fill_round(r, h / 2.0, bg)?;
    unsafe {
        text.layout.SetMaxWidth(w)?;
        text.layout.SetMaxHeight(h)?;
        text.layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        text.layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    }
    p.draw_text(&text, r.left, r.top, fg)
}

/// What a thumbnail or icon is made of.
pub struct ThumbSource {
    pub icon: Option<IWICBitmap>,
    pub badge: Option<Badge>,
}

impl Renderer {
    /// Reads the icon and badge of a PKG; `None` if it is not a PS4 PKG.
    /// `path` lets patches borrow the icon of the base game next to them.
    pub fn thumb_source<R: std::io::Read + std::io::Seek>(
        &self,
        reader: R,
        path: Option<&std::path::Path>,
    ) -> Option<ThumbSource> {
        let mut pkg = pkg_core::Ps4Pkg::open(reader).ok()?;
        let info = PkgInfo::read(&mut pkg).ok();
        let icon = match &info {
            Some(info) => pkg_core::artwork::load(&mut pkg, info, path, false).icon.map(|(b, _)| b),
            None => pkg.read_icon().ok().flatten(),
        };
        Some(ThumbSource {
            icon: icon.and_then(|b| self.decode(&b).ok()),
            badge: info.as_ref().and_then(Badge::for_info),
        })
    }
}
