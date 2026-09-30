//! A window that displays a [`CardModel`]: a child of the preview pane (or
//! Peek) inside the shell extension, or the top-level viewer window.
//!
//! The window owns its state (renderer, model, render target) through
//! `GWLP_USERDATA`; it is freed on `WM_NCDESTROY`.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use windows::Win32::Foundation::{
    COLORREF, D2DERR_RECREATE_TARGET, GetLastError, GlobalFree, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileSaveDialog, IFileSaveDialog, SIGDN_FILESYSPATH};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{HSTRING, PCWSTR, Result, w};

use crate::card::{CardModel, DeviceCache, Hit, HitTarget, Interaction};
use crate::Renderer;

const CLASS_NAME: PCWSTR = w!("PkgPeekCardWindow");
const TOAST_TIMER: usize = 1;
const TOAST_MS: u32 = 1400;
const CF_UNICODETEXT: u32 = 13;
// Declared in the Controls module, which this crate does not otherwise need.
const WM_MOUSELEAVE: u32 = 0x02A3;

// Context menu commands.
const CMD_COPY_TITLE_ID: usize = 1;
const CMD_COPY_CONTENT_ID: usize = 2;
const CMD_COPY_TITLE: usize = 3;
const CMD_SAVE_ICON: usize = 4;
const CMD_SAVE_BACKGROUND: usize = 5;
const CMD_OPEN_FOLDER: usize = 6;
const CMD_CLOSE: usize = 7;

struct CardWindow {
    renderer: Renderer,
    model: CardModel,
    path: Option<PathBuf>,
    /// Top-level viewer: Esc closes, closing quits the message loop.
    standalone: bool,
    rt: Option<ID2D1HwndRenderTarget>,
    cache: DeviceCache,
    hits: Vec<Hit>,
    ui: Interaction,
    tracking_mouse: bool,
}

/// Registers the window class for `hinstance` (idempotent).
pub fn register_class(hinstance: HINSTANCE, icon: Option<HICON>) -> Result<()> {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hIcon: icon.unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            let err = GetLastError();
            if err.0 != 1410 {
                // ERROR_CLASS_ALREADY_EXISTS is fine.
                return Err(err.into());
            }
        }
    }
    Ok(())
}

/// Unregisters the class; a DLL must do this before it can be unloaded.
pub fn unregister_class(hinstance: HINSTANCE) {
    unsafe {
        let _ = UnregisterClassW(CLASS_NAME, Some(hinstance));
    }
}

pub struct CardWindowParams<'a> {
    pub hinstance: HINSTANCE,
    pub parent: Option<HWND>,
    pub style: WINDOW_STYLE,
    pub ex_style: WINDOW_EX_STYLE,
    pub rect: RECT,
    pub title: &'a str,
    pub path: Option<PathBuf>,
    pub standalone: bool,
}

pub fn create_card_window(params: CardWindowParams, renderer: Renderer, model: CardModel) -> Result<HWND> {
    let state = Box::new(CardWindow {
        renderer,
        model,
        path: params.path,
        standalone: params.standalone,
        rt: None,
        cache: DeviceCache::default(),
        hits: Vec::new(),
        ui: Interaction::default(),
        tracking_mouse: false,
    });
    let r = params.rect;
    unsafe {
        CreateWindowExW(
            params.ex_style,
            CLASS_NAME,
            &HSTRING::from(params.title),
            params.style,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            params.parent,
            None,
            Some(params.hinstance),
            Some(Box::into_raw(state) as *const c_void),
        )
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CardWindow;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        // Modal loops (context menu, save dialog) dispatch messages back to
        // this window, so they run without holding a borrow of the state.
        if msg == WM_CONTEXTMENU {
            let _ = catch_unwind(AssertUnwindSafe(|| context_menu(hwnd, ptr, lparam)));
            return LRESULT(0);
        }
        match catch_unwind(AssertUnwindSafe(|| (*ptr).handle(hwnd, msg, wparam, lparam))) {
            Ok(Some(result)) => result,
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn loword(v: isize) -> i32 {
    (v & 0xFFFF) as i16 as i32
}

fn hiword(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as i16 as i32
}

fn dpi_of(hwnd: HWND) -> f32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96.0,
        dpi => dpi as f32,
    }
}

impl CardWindow {
    fn handle(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        unsafe {
            match msg {
                WM_ERASEBKGND => Some(LRESULT(1)),
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut ps);
                    if self.render(hwnd).is_err() {
                        self.rt = None;
                        self.cache.reset();
                    }
                    let _ = EndPaint(hwnd, &ps);
                    Some(LRESULT(0))
                }
                WM_SIZE => {
                    if let Some(rt) = &self.rt {
                        let size = D2D_SIZE_U { width: loword(lparam.0) as u32, height: hiword(lparam.0) as u32 };
                        if rt.Resize(&size).is_err() {
                            self.rt = None;
                            self.cache.reset();
                        }
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    Some(LRESULT(0))
                }
                WM_DPICHANGED => {
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    Some(LRESULT(0))
                }
                WM_DPICHANGED_AFTERPARENT => {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    Some(LRESULT(0))
                }
                WM_MOUSEMOVE => {
                    if !self.tracking_mouse {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        self.tracking_mouse = TrackMouseEvent(&mut tme).is_ok();
                    }
                    let hover = self.hit_at(hwnd, lparam).map(|h| h.target);
                    self.set_hover(hwnd, hover);
                    Some(LRESULT(0))
                }
                WM_MOUSELEAVE => {
                    self.tracking_mouse = false;
                    self.set_hover(hwnd, None);
                    Some(LRESULT(0))
                }
                WM_SETCURSOR => {
                    if loword(lparam.0) as u32 == HTCLIENT && self.ui.hover.is_some() {
                        if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
                            SetCursor(Some(hand));
                            return Some(LRESULT(1));
                        }
                    }
                    None
                }
                WM_LBUTTONUP => {
                    if let Some(hit) = self.hit_at(hwnd, lparam).cloned() {
                        self.copy(hwnd, &hit.text);
                    }
                    Some(LRESULT(0))
                }
                WM_TIMER if wparam.0 == TOAST_TIMER => {
                    let _ = KillTimer(Some(hwnd), TOAST_TIMER);
                    self.ui.toast = None;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    Some(LRESULT(0))
                }
                WM_KEYDOWN if self.standalone && wparam.0 == VK_ESCAPE.0 as usize => {
                    let _ = DestroyWindow(hwnd);
                    Some(LRESULT(0))
                }
                WM_DESTROY if self.standalone => {
                    PostQuitMessage(0);
                    Some(LRESULT(0))
                }
                _ => None,
            }
        }
    }

    fn render(&mut self, hwnd: HWND) -> Result<()> {
        unsafe {
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc)?;
            let (pw, ph) = ((rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32);
            if pw == 0 || ph == 0 {
                return Ok(());
            }
            let dpi = dpi_of(hwnd);
            if self.rt.is_none() {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: dpi,
                    dpiY: dpi,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                };
                let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd,
                    pixelSize: D2D_SIZE_U { width: pw, height: ph },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                };
                self.rt = Some(self.renderer.d2d_factory().CreateHwndRenderTarget(&props, &hwnd_props)?);
                self.cache.reset();
            }
            let Some(rt) = self.rt.clone() else { return Ok(()) };
            rt.SetDpi(dpi, dpi);
            let scale = 96.0 / dpi;
            rt.BeginDraw();
            let drawn =
                self.renderer.draw_card(&rt, &self.model, &mut self.cache, pw as f32 * scale, ph as f32 * scale, &self.ui);
            let ended = rt.EndDraw(None, None);
            if let Ok(hits) = drawn {
                self.hits = hits;
            }
            if let Err(e) = ended {
                if e.code() == D2DERR_RECREATE_TARGET {
                    self.rt = None;
                    self.cache.reset();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return Ok(());
                }
                return Err(e);
            }
            Ok(())
        }
    }

    fn hit_at(&self, hwnd: HWND, lparam: LPARAM) -> Option<&Hit> {
        let scale = 96.0 / dpi_of(hwnd);
        let (x, y) = (loword(lparam.0) as f32 * scale, hiword(lparam.0) as f32 * scale);
        self.hits.iter().find(|h| h.contains(x, y))
    }

    fn set_hover(&mut self, hwnd: HWND, hover: Option<HitTarget>) {
        if self.ui.hover != hover {
            self.ui.hover = hover;
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }

    fn copy(&mut self, hwnd: HWND, text: &str) {
        self.ui.toast = Some(match copy_to_clipboard(hwnd, text) {
            Ok(()) => format!("Copiado: {text}"),
            Err(_) => "No se pudo copiar".to_string(),
        });
        unsafe {
            SetTimer(Some(hwnd), TOAST_TIMER, TOAST_MS, None);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

fn copy_to_clipboard(hwnd: HWND, text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        OpenClipboard(Some(hwnd))?;
        let result = (|| {
            EmptyClipboard()?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                let _ = GlobalFree(Some(mem));
                return Err(windows_core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            let _ = GlobalUnlock(mem);
            if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))) {
                let _ = GlobalFree(Some(mem));
                return Err(e);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Shows the context menu. Runs outside `handle` because TrackPopupMenu and
/// the save dialog pump messages into this window.
unsafe fn context_menu(hwnd: HWND, ptr: *mut CardWindow, lparam: LPARAM) {
    unsafe {
        let (has_info, has_icon, has_bg, has_path, standalone) = {
            let s = &*ptr;
            (
                s.model.info.is_some(),
                s.model.icon_png.is_some(),
                s.model.background_png.is_some(),
                s.path.is_some(),
                s.standalone,
            )
        };
        let Ok(menu) = CreatePopupMenu() else { return };
        let add = |id: usize, text: PCWSTR, enabled: bool| {
            let flags = if enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
            let _ = AppendMenuW(menu, flags, id, text);
        };
        add(CMD_COPY_TITLE_ID, w!("Copiar Title ID"), has_info);
        add(CMD_COPY_CONTENT_ID, w!("Copiar Content ID"), has_info);
        add(CMD_COPY_TITLE, w!("Copiar título"), has_info);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        add(CMD_SAVE_ICON, w!("Guardar icono…"), has_icon);
        add(CMD_SAVE_BACKGROUND, w!("Guardar fondo…"), has_bg);
        if standalone {
            add(CMD_OPEN_FOLDER, w!("Mostrar en la carpeta"), has_path);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(CMD_CLOSE, w!("Cerrar\tEsc"), true);
        }

        let mut pt = POINT { x: loword(lparam.0), y: hiword(lparam.0) };
        if lparam.0 == -1 {
            // Keyboard (Shift+F10 / menu key): open at the window's corner.
            pt = POINT { x: 16, y: 16 };
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt);
        }
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);

        match cmd {
            CMD_COPY_TITLE_ID | CMD_COPY_CONTENT_ID | CMD_COPY_TITLE => {
                let s = &mut *ptr;
                let text = s.model.info.as_ref().map(|i| match cmd {
                    CMD_COPY_TITLE_ID => i.title_id.clone(),
                    CMD_COPY_CONTENT_ID => i.content_id.clone(),
                    _ => s.model.title.clone(),
                });
                if let Some(text) = text {
                    s.copy(hwnd, &text);
                }
            }
            CMD_SAVE_ICON | CMD_SAVE_BACKGROUND => {
                let (bytes, name) = {
                    let s = &*ptr;
                    let id = s.model.info.as_ref().map(|i| i.title_id.as_str()).unwrap_or("pkg");
                    if cmd == CMD_SAVE_ICON {
                        (s.model.icon_png.clone(), format!("{id}_icon0.png"))
                    } else {
                        (s.model.background_png.clone(), format!("{id}_pic1.png"))
                    }
                };
                if let Some(bytes) = bytes {
                    if let Some(target) = ask_save_path(hwnd, &name) {
                        let ok = std::fs::write(&target, bytes).is_ok();
                        let s = &mut *ptr;
                        s.ui.toast = Some(if ok { "Guardado".into() } else { "No se pudo guardar".into() });
                        SetTimer(Some(hwnd), TOAST_TIMER, TOAST_MS, None);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            CMD_OPEN_FOLDER => {
                if let Some(path) = &(*ptr).path {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("explorer.exe")
                        .raw_arg(format!("/select,\"{}\"", path.display()))
                        .spawn();
                }
            }
            CMD_CLOSE => {
                let _ = DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

fn ask_save_path(hwnd: HWND, default_name: &str) -> Option<String> {
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filters = [COMDLG_FILTERSPEC { pszName: w!("Imagen PNG"), pszSpec: w!("*.png") }];
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetDefaultExtension(w!("png")).ok()?;
        dialog.SetFileName(&HSTRING::from(default_name)).ok()?;
        dialog.Show(Some(hwnd)).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const c_void));
        path
    }
}

/// Dark title bar for the viewer window (the card itself is always dark).
pub fn use_dark_title_bar(hwnd: HWND) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    unsafe {
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const i32 as *const c_void, 4);
        let caption = COLORREF(0x0016110E); // BGR of the card base color
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &caption as *const COLORREF as *const c_void, 4);
    }
}
