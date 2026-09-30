//! IPreviewHandler: shows the card in Explorer's preview pane (Alt+P) and in
//! PowerToys Peek (Ctrl+Space), both of which host registered preview
//! handlers. The card itself lives in `pkg_render::window`.

use std::cell::RefCell;
use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use pkg_render::window::{CardWindowParams, create_card_window, register_class, unregister_class};
use pkg_render::{CardModel, Renderer, ui_languages};
use windows::Win32::Foundation::{COLORREF, E_FAIL, E_NOTIMPL, E_POINTER, E_UNEXPECTED, HWND, RECT, S_FALSE};
use windows::Win32::Graphics::Gdi::LOGFONTW;
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithFile_Impl};
use windows::Win32::UI::Shell::{
    IPreviewHandler, IPreviewHandler_Impl, IPreviewHandlerFrame, IPreviewHandlerVisuals, IPreviewHandlerVisuals_Impl,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SetParent, SetWindowPos, WINDOW_EX_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows_core::{BOOL, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement};

use crate::{ModuleRef, already_initialized, dll_instance, guard, path_from};

#[derive(Default)]
struct State {
    path: Option<PathBuf>,
    parent: HWND,
    rect: RECT,
    hwnd: Option<HWND>,
    site: Option<IUnknown>,
}

#[implement(IPreviewHandler, IInitializeWithFile, IObjectWithSite, IOleWindow, IPreviewHandlerVisuals)]
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

impl IInitializeWithFile_Impl for PreviewHandler_Impl {
    fn Initialize(&self, path: &PCWSTR, _mode: u32) -> Result<()> {
        guard(|| {
            let mut state = self.state.borrow_mut();
            if state.path.is_some() {
                return Err(already_initialized());
            }
            state.path = Some(path_from(path)?);
            Ok(())
        })
    }
}

impl IPreviewHandler_Impl for PreviewHandler_Impl {
    fn SetWindow(&self, parent: HWND, rect: *const RECT) -> Result<()> {
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
        guard(|| {
            let (path, parent, rect) = {
                let state = self.state.borrow();
                if state.hwnd.is_some() {
                    return Ok(());
                }
                (state.path.clone().ok_or(E_UNEXPECTED)?, state.parent, state.rect)
            };
            let renderer = Renderer::new()?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let model = match File::open(&path) {
                Ok(file) => {
                    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                    CardModel::load(&renderer, BufReader::new(file), Some(&path), name, len, &ui_languages())
                }
                Err(e) => CardModel::error(name, 0, format!("No se pudo abrir el archivo: {e}")),
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
                    path: Some(path),
                    standalone: false,
                },
                renderer,
                model,
            )?;
            self.state.borrow_mut().hwnd = Some(hwnd);
            Ok(())
        })
    }

    fn Unload(&self) -> Result<()> {
        guard(|| {
            self.destroy_window();
            self.state.borrow_mut().path = None;
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
