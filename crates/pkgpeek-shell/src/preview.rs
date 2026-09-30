//! IPreviewHandler: shows the card in Explorer's preview pane (Alt+P) and in
//! PowerToys Peek (Ctrl+Space), both of which host registered preview
//! handlers. The card itself lives in `pkg_render::window`.
//!
//! Explorer runs preview handlers in a low-integrity prevhost.exe and only
//! initializes them with a stream; hosts without that isolation may use a
//! path instead. Both are supported.

use std::cell::RefCell;
use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use pkg_render::window::{CardWindowParams, create_card_window, register_class, unregister_class};
use pkg_render::{CardModel, Renderer, ui_languages};
use windows::Win32::Foundation::{COLORREF, E_FAIL, E_NOTIMPL, E_POINTER, E_UNEXPECTED, HWND, RECT, S_FALSE};
use windows::Win32::Graphics::Gdi::LOGFONTW;
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::System::Com::IStream;
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithFile, IInitializeWithFile_Impl, IInitializeWithStream, IInitializeWithStream_Impl,
};
use windows::Win32::UI::Shell::{
    IPreviewHandler, IPreviewHandler_Impl, IPreviewHandlerFrame, IPreviewHandlerVisuals, IPreviewHandlerVisuals_Impl,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SetParent, SetWindowPos, WINDOW_EX_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows_core::{BOOL, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement};

use crate::stream::{self, StreamReader};
use crate::{ModuleRef, already_initialized, dll_instance, guard, path_from};

enum Source {
    File(PathBuf),
    Stream(IStream),
}

#[derive(Default)]
struct State {
    source: Option<Source>,
    parent: HWND,
    rect: RECT,
    hwnd: Option<HWND>,
    site: Option<IUnknown>,
}

// Not agile: windows-rs objects are agile (free-threaded marshaler) by default,
// which let prevhost call DoPreview from an RPC worker thread. The card window
// then lived on a thread with no message loop: it never painted, and Explorer
// hung when it resized it. As a plain STA object every call arrives on the
// host's UI thread.
#[implement(
    IPreviewHandler,
    IInitializeWithStream,
    IInitializeWithFile,
    IObjectWithSite,
    IOleWindow,
    IPreviewHandlerVisuals,
    Agile = false
)]
pub struct PreviewHandler {
    state: RefCell<State>,
    _module: ModuleRef,
}

impl PreviewHandler {
    pub fn new() -> Self {
        Self { state: RefCell::new(State::default()), _module: ModuleRef::new() }
    }

    fn destroy_window(&self) {
        let hwnd = self.state.borrow_mut().hwnd.take();
        if let Some(hwnd) = hwnd {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            unregister_class(dll_instance());
        }
    }
}

impl Drop for PreviewHandler {
    fn drop(&mut self) {
        self.destroy_window();
    }
}

fn move_to(hwnd: HWND, r: &RECT) {
    unsafe {
        let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

impl PreviewHandler {
    fn set_source(&self, source: Source) -> Result<()> {
        let mut state = self.state.borrow_mut();
        if state.source.is_some() {
            return Err(already_initialized());
        }
        state.source = Some(source);
        Ok(())
    }

    fn do_preview(&self) -> Result<()> {
        guard(|| {
            let (parent, rect) = {
                let state = self.state.borrow();
                if state.hwnd.is_some() {
                    return Ok(());
                }
                (state.parent, state.rect)
            };
            let renderer = Renderer::new()?;
            let (model, path) = {
                let state = self.state.borrow();
                let source = state.source.as_ref().ok_or(E_UNEXPECTED)?;
                load_model(&renderer, source)
            };
            let hinstance = dll_instance();
            register_class(hinstance, None)?;
            let hwnd = create_card_window(
                CardWindowParams {
                    hinstance,
                    parent: Some(parent),
                    style: WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                    ex_style: WINDOW_EX_STYLE::default(),
                    rect,
                    title: "",
                    path,
                    standalone: false,
                },
                renderer,
                model,
            )?;
            self.state.borrow_mut().hwnd = Some(hwnd);
            Ok(())
        })
    }
}

impl IInitializeWithStream_Impl for PreviewHandler_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        pkg_render::debug::log("preview: IInitializeWithStream");
        guard(|| self.set_source(Source::Stream(stream.ok()?.clone())))
    }
}

impl IInitializeWithFile_Impl for PreviewHandler_Impl {
    fn Initialize(&self, path: &PCWSTR, _mode: u32) -> Result<()> {
        pkg_render::debug::log("preview: IInitializeWithFile");
        guard(|| self.set_source(Source::File(path_from(path)?)))
    }
}

/// Reads the PKG from whichever source the host gave us.
fn load_model(renderer: &Renderer, source: &Source) -> (CardModel, Option<PathBuf>) {
    let languages = ui_languages();
    match source {
        Source::File(path) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let model = match File::open(path) {
                Ok(file) => {
                    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                    CardModel::load(renderer, BufReader::new(file), Some(path), name, len, &languages)
                }
                Err(e) => CardModel::error(name, 0, format!("No se pudo abrir el archivo: {e}")),
            };
            (model, Some(path.clone()))
        }
        Source::Stream(stream) => {
            let (name, len) = stream::stat(stream);
            // A full path (what shell file streams report) enables borrowing
            // artwork from the base game next to a patch.
            let path = name.as_deref().map(PathBuf::from).filter(|p| p.is_absolute() && p.exists());
            let file_name = name
                .as_deref()
                .map(|n| Path::new(n).file_name().map_or(n.to_string(), |f| f.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let reader = BufReader::with_capacity(64 * 1024, StreamReader(stream.clone()));
            let model = CardModel::load(renderer, reader, path.as_deref(), file_name, len, &languages);
            (model, path)
        }
    }
}

impl IPreviewHandler_Impl for PreviewHandler_Impl {
    fn SetWindow(&self, parent: HWND, rect: *const RECT) -> Result<()> {
        pkg_render::debug::log(format!("preview: SetWindow {parent:?} {:?}", unsafe { rect.as_ref() }));
        guard(|| {
            if rect.is_null() {
                return Err(E_POINTER.into());
            }
            let (hwnd, r) = {
                let mut state = self.state.borrow_mut();
                state.parent = parent;
                state.rect = unsafe { *rect };
                (state.hwnd, state.rect)
            };
            if let Some(hwnd) = hwnd {
                unsafe {
                    let _ = SetParent(hwnd, Some(parent));
                }
                move_to(hwnd, &r);
            }
            Ok(())
        })
    }

    fn SetRect(&self, rect: *const RECT) -> Result<()> {
        guard(|| {
            if rect.is_null() {
                return Err(E_POINTER.into());
            }
            let (hwnd, r) = {
                let mut state = self.state.borrow_mut();
                state.rect = unsafe { *rect };
                (state.hwnd, state.rect)
            };
            if let Some(hwnd) = hwnd {
                move_to(hwnd, &r);
            }
            Ok(())
        })
    }

    fn DoPreview(&self) -> Result<()> {
        pkg_render::debug::log("preview: DoPreview");
        let result = self.do_preview();
        pkg_render::debug::log(format!("preview: DoPreview -> {result:?}"));
        result
    }

    fn Unload(&self) -> Result<()> {
        pkg_render::debug::log("preview: Unload");
        guard(|| {
            self.destroy_window();
            self.state.borrow_mut().source = None;
            Ok(())
        })
    }

    fn SetFocus(&self) -> Result<()> {
        if let Some(hwnd) = self.state.borrow().hwnd {
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        }
        Ok(())
    }

    fn QueryFocus(&self) -> Result<HWND> {
        let hwnd = unsafe { GetFocus() };
        if hwnd.is_invalid() { Err(E_FAIL.into()) } else { Ok(hwnd) }
    }

    fn TranslateAccelerator(&self, msg: *const MSG) -> Result<()> {
        let frame = self.state.borrow().site.as_ref().and_then(|s| s.cast::<IPreviewHandlerFrame>().ok());
        match frame {
            Some(frame) => unsafe { frame.TranslateAccelerator(msg) },
            None => Err(windows_core::Error::from_hresult(S_FALSE)),
        }
    }
}

impl IObjectWithSite_Impl for PreviewHandler_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> Result<()> {
        self.state.borrow_mut().site = site.cloned();
        Ok(())
    }

    fn GetSite(&self, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        if ppv.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppv = std::ptr::null_mut() };
        match &self.state.borrow().site {
            Some(site) => unsafe { site.query(riid, ppv).ok() },
            None => Err(E_FAIL.into()),
        }
    }
}

impl IOleWindow_Impl for PreviewHandler_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        self.state.borrow().hwnd.ok_or_else(|| E_FAIL.into())
    }

    fn ContextSensitiveHelp(&self, _enter: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

/// The card has its own dark look; the host's colors and font are ignored.
impl IPreviewHandlerVisuals_Impl for PreviewHandler_Impl {
    fn SetBackgroundColor(&self, _color: COLORREF) -> Result<()> {
        Ok(())
    }

    fn SetFont(&self, _font: *const LOGFONTW) -> Result<()> {
        Ok(())
    }

    fn SetTextColor(&self, _color: COLORREF) -> Result<()> {
        Ok(())
    }
}
