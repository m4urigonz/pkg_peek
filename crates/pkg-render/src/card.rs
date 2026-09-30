//! The preview card shown by Explorer's preview pane, PowerToys Peek and the
//! viewer window. Layout adapts to the available size: a vertical "narrow"
//! layout for the preview pane and a "wide" one with the background art.

use std::io::{Read, Seek};
use std::path::Path;

use pkg_core::{Category, IconSource, PkgInfo, Ps4Pkg, artwork};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1RenderTarget};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_RANGE,
};
use windows::Win32::Graphics::Imaging::{GUID_WICPixelFormat32bppPBGRA, IWICBitmap, WICBitmapCacheOnLoad};
use windows_core::{Interface, Result};
use windows_numerics::Matrix3x2;

use crate::draw::{Font, Painter, Text, Wrap, contains, mix, rect, rgb, rgba, wic_target, with_alpha};
use crate::locale::{format_date, format_size};
use crate::thumb::draw_placeholder;
use crate::Renderer;

const BASE: D2D1_COLOR_F = rgb(0x0E1116);
const TEXT: D2D1_COLOR_F = rgb(0xFFFFFF);
const MUTED: D2D1_COLOR_F = rgba(0xFFFFFF, 0.62);
const LINK: D2D1_COLOR_F = rgb(0x6CB6FF);
const CHIP: D2D1_COLOR_F = rgba(0xFFFFFF, 0.14);
const PS_BLUE: D2D1_COLOR_F = rgb(0x0070D1);
const WARNING: D2D1_COLOR_F = rgb(0xE5484D);

/// Width (DIPs) from which the wide layout is used.
const WIDE_MIN_WIDTH: f32 = 600.0;

/// Everything the card shows, decoded once per file.
pub struct CardModel {
    pub file_name: String,
    pub file_size: u64,
    /// Title in the user's language when the PKG has it.
    pub title: String,
    pub info: Option<PkgInfo>,
    /// Why `info` is missing.
    pub error: Option<String>,
    pub icon: Option<IWICBitmap>,
    pub icon_source: Option<IconSource>,
    pub icon_png: Option<Vec<u8>>,
    pub background: Option<IWICBitmap>,
    pub background_png: Option<Vec<u8>>,
    accent: Option<D2D1_COLOR_F>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    TitleId,
    ContentId,
}

/// Clickable area of the card (value that can be copied).
#[derive(Debug, Clone)]
pub struct Hit {
    pub rect: D2D_RECT_F,
    pub target: HitTarget,
    pub text: String,
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        contains(&self.rect, x, y)
    }
}

/// Transient UI state drawn over the card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interaction {
    pub hover: Option<HitTarget>,
    pub toast: Option<String>,
}

/// Direct2D bitmaps for the current render target. Call [`DeviceCache::reset`]
/// when the target or the model changes.
#[derive(Default)]
pub struct DeviceCache {
    icon: Option<ID2D1Bitmap>,
    background: Option<ID2D1Bitmap>,
}

impl DeviceCache {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

impl CardModel {
    pub fn error(file_name: String, file_size: u64, message: String) -> Self {
        Self {
            title: file_name.clone(),
            file_name,
            file_size,
            info: None,
            error: Some(message),
            icon: None,
            icon_source: None,
            icon_png: None,
            background: None,
            background_png: None,
            accent: None,
        }
    }

    /// Reads a PKG. `path` lets patches borrow artwork from the base game in
    /// the same folder. Never fails: problems end up in `error`.
    pub fn load<R: Read + Seek>(
        r: &Renderer,
        reader: R,
        path: Option<&Path>,
        file_name: String,
        file_size: u64,
        languages: &[String],
    ) -> Self {
        let mut pkg = match Ps4Pkg::open(reader) {
            Ok(p) => p,
            Err(e) => return Self::error(file_name, file_size, e.to_string()),
        };
        let info = match PkgInfo::read(&mut pkg) {
            Ok(i) => i,
            Err(e) => return Self::error(file_name, file_size, e.to_string()),
        };
        let art = artwork::load(&mut pkg, &info, path, true);
        let mut model = Self::error(file_name, pkg.file_len(), String::new());
        model.error = None;
        if let Some((bytes, source)) = art.icon {
            model.icon = r.decode(&bytes).ok();
            model.icon_source = Some(source);
            model.icon_png = Some(bytes);
        }
        if let Some(bytes) = art.background {
            model.background = r.decode(&bytes).ok();
            model.background_png = Some(bytes);
        }
        model.accent = model.icon.as_ref().and_then(|i| r.average_color(&i.cast().ok()?));
        model.title = info.title_for(languages).to_string();
        model.info = Some(info);
        model
    }

    pub fn copy_text(&self, target: HitTarget) -> Option<&str> {
        let info = self.info.as_ref()?;
        Some(match target {
            HitTarget::TitleId => &info.title_id,
            HitTarget::ContentId => &info.content_id,
        })
    }
}

struct Field {
    label: &'static str,
    value: String,
    full_row: bool,
    hit: Option<HitTarget>,
}

impl Field {
    fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self { label, value: value.into(), full_row: false, hit: None }
    }
}

fn fields(info: &PkgInfo, file_size: u64) -> Vec<Field> {
    let mut f = vec![Field { hit: Some(HitTarget::TitleId), ..Field::new("Title ID", &info.title_id) }];
    if let Some(v) = &info.app_ver {
        f.push(Field::new("Versión app", v));
    }
    if let Some(v) = &info.version {
        f.push(Field::new("Versión maestra", v));
    }
    if let Some(fw) = info.firmware() {
        f.push(Field::new("Firmware requerido", fw));
    }
    f.push(Field::new("Tamaño", format_size(file_size)));
    if let Some(d) = &info.creation_date {
        f.push(Field::new("Fecha de creación", format_date(d)));
    }
    if let Some(region) = info.region {
        f.push(Field::new("Región", region.label()));
    }
    if let Some(sdk) = &info.sdk_ver {
        f.push(Field::new("SDK", sdk));
    }
    f.push(Field {
        full_row: true,
        hit: Some(HitTarget::ContentId),
        ..Field::new("Content ID", &info.content_id)
    });
    f
}

fn chips(info: &PkgInfo) -> Vec<(String, D2D1_COLOR_F, D2D1_COLOR_F)> {
    let mut c = vec![("PS4".to_string(), PS_BLUE, TEXT)];
    let (bg, fg) = match info.category {
        Category::Patch => (rgb(0xF5A623), rgb(0x231500)),
        Category::Dlc => (rgb(0x8E5CF7), TEXT),
        Category::App => (rgb(0x138A83), TEXT),
        _ => (CHIP, TEXT),
    };
    let label = match (&info.category, info.short_app_ver()) {
        (Category::Patch, Some(v)) => format!("Actualización {v}"),
        (cat, _) => cat.label().to_string(),
    };
    c.push((label, bg, fg));
    if let Some(region) = info.region {
        c.push((region.code().to_string(), CHIP, TEXT));
    }
    c
}

impl Renderer {
    /// Renders the card into a new bitmap (`width`x`height` DIPs at `scale`).
    pub fn render_card(&self, model: &CardModel, width: u32, height: u32, scale: f32) -> Result<IWICBitmap> {
        let (pw, ph) = ((width as f32 * scale).round() as u32, (height as f32 * scale).round() as u32);
        let target = unsafe { self.wic.CreateBitmap(pw, ph, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)? };
        let rt = wic_target(self, &target, 96.0 * scale)?;
        let mut cache = DeviceCache::default();
        unsafe { rt.BeginDraw() };
        self.draw_card(&rt, model, &mut cache, width as f32, height as f32, &Interaction::default())?;
        unsafe { rt.EndDraw(None, None)? };
        Ok(target)
    }

    /// Draws the card between BeginDraw/EndDraw (managed by the caller).
    /// Sizes are in DIPs. Returns the clickable areas.
    pub fn draw_card(
        &self,
        rt: &ID2D1RenderTarget,
        model: &CardModel,
        cache: &mut DeviceCache,
        w: f32,
        h: f32,
        ui: &Interaction,
    ) -> Result<Vec<Hit>> {
        let p = Painter::new(self, rt);
        p.fill(rect(0.0, 0.0, w, h), BASE)?;
        if cache.icon.is_none() {
            cache.icon = model.icon.as_ref().map(|i| p.bitmap(i)).transpose().ok().flatten();
        }
        if cache.background.is_none() {
            cache.background = model.background.as_ref().map(|i| p.bitmap(i)).transpose().ok().flatten();
        }

        let mut hits = Vec::new();
        match &model.info {
            None => draw_error(&p, model, w, h)?,
            Some(info) if w >= WIDE_MIN_WIDTH && h >= 240.0 => draw_wide(&p, model, info, cache, w, h, ui, &mut hits)?,
            Some(info) => draw_narrow(&p, model, info, cache, w, h, ui, &mut hits)?,
        }
        if let Some(toast) = &ui.toast {
            draw_toast(&p, toast, w, h)?;
        }
        Ok(hits)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_wide(
    p: &Painter,
    model: &CardModel,
    info: &PkgInfo,
    cache: &DeviceCache,
    w: f32,
    h: f32,
    ui: &Interaction,
    hits: &mut Vec<Hit>,
) -> Result<()> {
    let full = rect(0.0, 0.0, w, h);
    if let Some(bg) = &cache.background {
        p.image_cover(bg, full, 1.0)?;
        p.gradient(full, (0.0, 0.0), (w, 0.0), &[
            (0.0, with_alpha(BASE, 0.94)),
            (0.55, with_alpha(BASE, 0.82)),
            (1.0, with_alpha(BASE, 0.55)),
        ])?;
        p.gradient(full, (0.0, 0.0), (0.0, h), &[(0.55, with_alpha(BASE, 0.0)), (1.0, with_alpha(BASE, 0.9))])?;
    } else if let Some(accent) = model.accent {
        p.gradient(full, (0.0, 0.0), (w * 0.8, h), &[(0.0, mix(BASE, accent, 0.38)), (1.0, BASE)])?;
    }

    let pad = 32.0;
    let icon = (h * 0.4).min(w * 0.26).clamp(96.0, 240.0).round();
    draw_icon(p, cache, model, pad, pad, icon)?;

    let x = pad + icon + 30.0;
    let cw = w - x - pad;
    let mut y = pad - 6.0;
    y = draw_heading(p, model, info, x, y, cw, 30.0)?;
    y = draw_warning(p, info, x, y, cw)?;
    let cols = if cw >= 540.0 { 3 } else { 2 };
    y = draw_fields(p, &fields(info, model.file_size), x, y, cw, cols, ui, hits)?;
    draw_notes(p, info, x, y + 22.0, cw, h - pad)
}

#[allow(clippy::too_many_arguments)]
fn draw_narrow(
    p: &Painter,
    model: &CardModel,
    info: &PkgInfo,
    cache: &DeviceCache,
    w: f32,
    h: f32,
    ui: &Interaction,
    hits: &mut Vec<Hit>,
) -> Result<()> {
    let pad = 18.0;
    let banner_h = (w * 9.0 / 16.0).min(h * 0.36).max(80.0).round();
    let banner = rect(0.0, 0.0, w, banner_h);
    if let Some(bg) = &cache.background {
        p.image_cover(bg, banner, 1.0)?;
    } else if let Some(accent) = model.accent {
        p.gradient(banner, (0.0, 0.0), (w, banner_h), &[(0.0, mix(BASE, accent, 0.55)), (1.0, mix(BASE, accent, 0.2))])?;
    }
    p.gradient(banner, (0.0, 0.0), (0.0, banner_h), &[(0.0, with_alpha(BASE, 0.05)), (1.0, BASE)])?;

    let icon = (w * 0.28).clamp(64.0, 128.0).round();
    let (ix, iy) = (pad, (banner_h - icon * 0.62).round());
    let ring = 3.0;
    p.fill_round(
        rect(ix - ring, iy - ring, ix + icon + ring, iy + icon + ring),
        (icon * 0.07).round() + ring,
        BASE,
    )?;
    draw_icon(p, cache, model, ix, iy, icon)?;

    let cw = w - 2.0 * pad;
    let mut y = iy + icon + 14.0;
    y = draw_heading(p, model, info, pad, y, cw, 21.0)?;
    y = draw_warning(p, info, pad, y, cw)?;
    let cols = if cw >= 300.0 { 2 } else { 1 };
    y = draw_fields(p, &fields(info, model.file_size), pad, y, cw, cols, ui, hits)?;
    draw_notes(p, info, pad, y + 20.0, cw, h - pad)
}

fn draw_icon(p: &Painter, cache: &DeviceCache, model: &CardModel, x: f32, y: f32, size: f32) -> Result<()> {
    let radius = (size * 0.07).round();
    match &cache.icon {
        Some(bmp) => {
            let px = unsafe { bmp.GetPixelSize() };
            let (iw, ih) = (px.width.max(1) as f32, px.height.max(1) as f32);
            // Trophy icons are 320x176: show them letterboxed on a tile.
            let square = model.icon_source != Some(IconSource::Trophy) && (iw - ih).abs() < 1.0;
            if square {
                p.image(bmp, rect(x, y, x + size, y + size), None, 1.0, radius)?;
            } else {
                p.fill_round(rect(x, y, x + size, y + size), radius, rgb(0x1B2028))?;
                let fit = (size / iw).min(size / ih);
                let (dw, dh) = (iw * fit, ih * fit);
                let (dx, dy) = (x + (size - dw) / 2.0, y + (size - dh) / 2.0);
                p.image(bmp, rect(dx, dy, dx + dw, dy + dh), None, 1.0, 0.0)?;
            }
        }
        None => unsafe {
            p.rt.SetTransform(&Matrix3x2::translation(x, y));
            let r = draw_placeholder(p, size);
            p.rt.SetTransform(&Matrix3x2::identity());
            r?;
        },
    }
    p.stroke_round(rect(x + 0.5, y + 0.5, x + size - 0.5, y + size - 0.5), radius, rgba(0xFFFFFF, 0.10), 1.0)
}

/// Title and chips. Returns the next y.
fn draw_heading(p: &Painter, model: &CardModel, info: &PkgInfo, x: f32, y: f32, width: f32, size: f32) -> Result<f32> {
    let title = p.text(&model.title, Font::title(size), width, Wrap::Words(2))?;
    p.draw_text(&title, x, y, TEXT)?;
    let mut y = y + title.height + 10.0;

    let h = 26.0;
    let gap = 6.0;
    let mut cx = x;
    for (label, bg, fg) in chips(info) {
        let t = p.text(&label, Font::semibold(12.5), width, Wrap::None)?;
        let cw = (t.width + 22.0).round();
        if cx + cw > x + width && cx > x {
            cx = x;
            y += h + gap;
        }
        p.fill_round(rect(cx, y, cx + cw, y + h), h / 2.0, bg)?;
        center(&t, cw, h)?;
        p.draw_text(&t, cx, y, fg)?;
        cx += cw + gap;
    }
    Ok(y + h + 18.0)
}

fn draw_warning(p: &Painter, info: &PkgInfo, x: f32, y: f32, width: f32) -> Result<f32> {
    let Some(missing) = info.missing_bytes() else {
        return Ok(y);
    };
    let h = 34.0;
    let text = format!("Descarga incompleta · faltan {}", format_size(missing));
    let t = p.text(&text, Font::semibold(13.5), width - 24.0, Wrap::None)?;
    p.fill_round(rect(x, y, x + width, y + h), 8.0, with_alpha(WARNING, 0.92))?;
    p.draw_text(&t, x + 12.0, y + (h - t.height) / 2.0, TEXT)?;
    Ok(y + h + 16.0)
}

/// Label/value grid. Returns the y below the last row.
#[allow(clippy::too_many_arguments)]
fn draw_fields(
    p: &Painter,
    fields: &[Field],
    x: f32,
    y: f32,
    width: f32,
    cols: usize,
    ui: &Interaction,
    hits: &mut Vec<Hit>,
) -> Result<f32> {
    let (gap_x, gap_y) = (22.0, 14.0);
    let col_w = (width - gap_x * (cols - 1) as f32) / cols as f32;
    let (mut col, mut row_y, mut row_h) = (0, y, 0.0f32);
    for f in fields {
        let span = if f.full_row { cols } else { 1 };
        if col + span > cols {
            row_y += row_h + gap_y;
            (col, row_h) = (0, 0.0);
        }
        let cx = x + col as f32 * (col_w + gap_x);
        let cw = col_w * span as f32 + gap_x * (span - 1) as f32;
        let label = p.text(f.label, Font::text(12.0), cw, Wrap::None)?;
        let wrap = if f.full_row { Wrap::Chars(2) } else { Wrap::None };
        let value = p.text(&f.value, Font::semibold(15.0), cw, wrap)?;
        let hovered = f.hit.is_some() && ui.hover == f.hit;
        if hovered {
            let range = DWRITE_TEXT_RANGE { startPosition: 0, length: f.value.encode_utf16().count() as u32 };
            unsafe { value.layout.SetUnderline(true, range)? };
        }
        p.draw_text(&label, cx, row_y, MUTED)?;
        let vy = row_y + label.height + 1.0;
        p.draw_text(&value, cx, vy, if hovered { LINK } else { TEXT })?;
        if let Some(target) = f.hit {
            hits.push(Hit { rect: rect(cx, vy, cx + value.width, vy + value.height), target, text: f.value.clone() });
        }
        row_h = row_h.max(label.height + 1.0 + value.height);
        col += span;
        if col >= cols {
            row_y += row_h + gap_y;
            (col, row_h) = (0, 0.0);
        }
    }
    if col > 0 {
        row_y += row_h + gap_y;
    }
    Ok(row_y - gap_y)
}

/// Patch notes, as many lines as fit above `bottom`.
fn draw_notes(p: &Painter, info: &PkgInfo, x: f32, y: f32, width: f32, bottom: f32) -> Result<()> {
    let Some(notes) = &info.change_notes else {
        return Ok(());
    };
    let heading_font = Font::semibold(14.0);
    let body_font = Font::text(13.5);
    let lines = ((bottom - y - heading_font.line_height() - 6.0) / body_font.line_height()).floor();
    if lines < 2.0 {
        return Ok(());
    }
    p.fill(rect(x, y - 11.0, x + width, y - 10.0), rgba(0xFFFFFF, 0.10))?;
    let heading = match (&info.category, info.short_app_ver()) {
        (Category::Patch, Some(v)) => format!("Novedades de la versión {v}"),
        _ => "Novedades".to_string(),
    };
    let h = p.text(&heading, heading_font, width, Wrap::None)?;
    p.draw_text(&h, x, y, TEXT)?;
    let body = p.text(notes, body_font, width, Wrap::Words(lines as u32))?;
    p.draw_text(&body, x, y + h.height + 6.0, MUTED)
}

fn draw_error(p: &Painter, model: &CardModel, w: f32, h: f32) -> Result<()> {
    let pad = 24.0;
    let s = (w.min(h) * 0.26).clamp(56.0, 112.0).round();
    let message = model.error.as_deref().unwrap_or("No se pudo leer el archivo");
    let title = p.text(message, Font::semibold(17.0), w - 2.0 * pad, Wrap::Words(2))?;
    let detail = format!("{} · {}", model.file_name, format_size(model.file_size));
    let sub = p.text(&detail, Font::text(13.0), w - 2.0 * pad, Wrap::Chars(3))?;
    let total = s + 18.0 + title.height + 6.0 + sub.height;
    let y0 = ((h - total) / 2.0).max(pad).round();
    unsafe {
        p.rt.SetTransform(&Matrix3x2::translation(((w - s) / 2.0).round(), y0));
        let r = draw_placeholder(p, s);
        p.rt.SetTransform(&Matrix3x2::identity());
        r?;
        title.layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        sub.layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
    }
    let y = y0 + s + 18.0;
    p.draw_text(&title, pad, y, TEXT)?;
    p.draw_text(&sub, pad, y + title.height + 6.0, MUTED)
}

fn draw_toast(p: &Painter, text: &str, w: f32, h: f32) -> Result<()> {
    let t = p.text(text, Font::semibold(13.0), w - 48.0, Wrap::None)?;
    let (tw, th) = ((t.width + 32.0).round(), 36.0);
    let r = rect(((w - tw) / 2.0).round(), h - 20.0 - th, ((w + tw) / 2.0).round(), h - 20.0);
    p.fill_round(r, th / 2.0, rgba(0x1F2630, 0.96))?;
    p.stroke_round(r, th / 2.0, rgba(0xFFFFFF, 0.14), 1.0)?;
    center(&t, tw, th)?;
    p.draw_text(&t, r.left, r.top, TEXT)
}

/// Centers a single-line text inside a `w`x`h` box drawn at the box origin.
fn center(t: &Text, w: f32, h: f32) -> Result<()> {
    unsafe {
        t.layout.SetMaxWidth(w)?;
        t.layout.SetMaxHeight(h)?;
        t.layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        t.layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)
    }
}
