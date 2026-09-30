#![windows_subsystem = "windows"]
//! PKG Peek viewer window: opened by double-clicking a .pkg (or with a file
//! picker when started without arguments).

use std::ffi::OsString;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use pkg_render::window::{CardWindowParams, create_card_window, register_class, use_dark_title_bar};
use pkg_render::{CardModel, Renderer, ui_languages};
use windows::Win32::Foundation::{HINSTANCE, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{PCWSTR, w};

/// Default size of the viewer, in DIPs.
const WIDTH: i32 = 980;
const HEIGHT: i32 = 600;

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
            return;
        }
    }
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    let Some(path) = arg.or_else(ask_open_path) else { return };
    if let Err(e) = run(&path) {
        let text = windows_core::HSTRING::from(format!("No se pudo abrir el visor:\n{e}"));
        unsafe { MessageBoxW(None, &text, w!("PKG Peek"), MB_ICONERROR) };
    }
}

fn run(path: &Path) -> windows_core::Result<()> {
    let renderer = Renderer::new()?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let model = match File::open(path) {
        Ok(file) => {
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            CardModel::load(&renderer, BufReader::new(file), Some(path), name, len, &ui_languages())
        }
        Err(e) => CardModel::error(name, 0, format!("No se pudo abrir el archivo: {e}")),
    };
    let title = format!("{} — PKG Peek", model.title);

    unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
        // Icon resource 1, embedded by build.rs.
        let icon = LoadIconW(Some(hinstance), PCWSTR(1 as _)).ok();
        register_class(hinstance, icon)?;
        let hwnd = create_card_window(
            CardWindowParams {
                hinstance,
                parent: None,
                style: WS_OVERLAPPEDWINDOW,
                ex_style: WINDOW_EX_STYLE::default(),
                rect: centered_rect(),
                title: &title,
                path: Some(path.to_path_buf()),
                standalone: true,
            },
            renderer,
            model,
        )?;
        use_dark_title_bar(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Default-size window centered on the monitor under the mouse.
fn centered_rect() -> RECT {
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return RECT { left: CW_USEDEFAULT, top: CW_USEDEFAULT, right: CW_USEDEFAULT + WIDTH, bottom: CW_USEDEFAULT + HEIGHT };
        }
        let work = info.rcWork;
        let w = (WIDTH * dpi_x as i32 / 96).min(work.right - work.left);
        let h = (HEIGHT * dpi_x as i32 / 96).min(work.bottom - work.top);
        let left = work.left + (work.right - work.left - w) / 2;
        let top = work.top + (work.bottom - work.top - h) / 2;
        RECT { left, top, right: left + w, bottom: top + h }
    }
}

fn ask_open_path() -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filters = [
            COMDLG_FILTERSPEC { pszName: w!("Paquetes PS4 (*.pkg)"), pszSpec: w!("*.pkg") },
            COMDLG_FILTERSPEC { pszName: w!("Todos los archivos"), pszSpec: w!("*.*") },
        ];
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetTitle(w!("Abrir PKG")).ok()?;
        dialog.Show(None).ok()?;
        let name = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(OsString::from).map(PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const _));
        path
    }
}
