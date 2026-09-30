//! `pkgpeek-cli shelltest <dll> <pkg> <outdir>`: loads the shell extension
//! with LoadLibrary (no registration needed) and drives each COM handler the
//! way Explorer does, saving what they produce.

use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use pkg_render::Renderer;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDC, HBITMAP, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::Com::{IClassFactory, STGM_READ};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream, IPropertyStore};
use windows::Win32::UI::Shell::{IExtractIconW, IPreviewHandler, IThumbnailProvider, SHCreateStreamOnFileEx, WTS_ALPHATYPE};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::System::Com::IPersistFile;
use windows_core::{GUID, HRESULT, HSTRING, IUnknown, Interface, Result, s, w};

const CLSID_THUMBNAIL: GUID = GUID::from_u128(0x07c50bd9_7523_403d_ac91_9be7928d487a);
const CLSID_ICON: GUID = GUID::from_u128(0x7f6cbbfa_1bd6_4a26_a3d5_4599769ea694);
const CLSID_PREVIEW: GUID = GUID::from_u128(0x9f466039_090d_411e_b895_4bc797935442);
const CLSID_PROPERTIES: GUID = GUID::from_u128(0x1aae8d6e_7482_48a1_9b98_f7593c25292f);

type GetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

pub fn run(r: &Renderer, dll: &str, pkg: &str, out: &Path) -> Result<()> {
    unsafe {
        let module = LoadLibraryW(&HSTRING::from(dll))?;
        let proc = GetProcAddress(module, s!("DllGetClassObject")).ok_or_else(windows_core::Error::from_thread)?;
        let get: GetClassObject = std::mem::transmute(proc);
        let create = |clsid: GUID| -> Result<IUnknown> {
            let mut raw = std::ptr::null_mut();
            get(&clsid, &IClassFactory::IID, &mut raw).ok()?;
            IClassFactory::from_raw(raw).CreateInstance(None)
        };
        let path = HSTRING::from(pkg);
        let save = |name: &str| out.join(name).to_string_lossy().into_owned();

        // Thumbnail provider.
        let t = Instant::now();
        let thumb: IThumbnailProvider = create(CLSID_THUMBNAIL)?.cast()?;
        thumb.cast::<IInitializeWithFile>()?.Initialize(&path, STGM_READ.0)?;
        let (mut hbmp, mut alpha) = (HBITMAP::default(), WTS_ALPHATYPE::default());
        match thumb.GetThumbnail(256, &mut hbmp, &mut alpha) {
            Ok(()) => {
                r.save_hbitmap_png(hbmp, &save("shell_thumb.png"))?;
                let _ = DeleteObject(hbmp.into());
                println!("thumbnail: ok alpha={} ({:?})", alpha.0, t.elapsed());
            }
            Err(e) => println!("thumbnail: {e} ({:?})", t.elapsed()),
        }

        // Icon handler.
        let t = Instant::now();
        let icon: IExtractIconW = create(CLSID_ICON)?.cast()?;
        icon.cast::<IPersistFile>()?.Load(&path, STGM_READ)?;
        let mut buf = [0u16; 520];
        let (mut index, mut flags) = (0i32, 0u32);
        let pending = icon.GetIconLocation(0x20, &mut buf, &mut index, &mut flags);
        println!("icon GIL_ASYNC: {:?} (E_PENDING expected)", pending.err().map(|e| e.code()));
        icon.GetIconLocation(0, &mut buf, &mut index, &mut flags)?;
        let location = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]);
        println!("icon location: {location} index={index} flags={flags:#x}");
        let (mut large, mut small) = (HICON::default(), HICON::default());
        let location_w = HSTRING::from(location.as_str());
        // Without GIL_NOTFILENAME (0x8) Explorer extracts from the location itself.
        if flags & 0x8 == 0 {
            println!("icon extract: skipped, the shell reads icon {index} of {location}");
        } else { match icon.Extract(&location_w, index as u32, Some(&mut large), Some(&mut small), (16 << 16) | 48) {
            Ok(()) => {
                r.save_hicon_png(large, &save("shell_icon48.png"))?;
                r.save_hicon_png(small, &save("shell_icon16.png"))?;
                let _ = DestroyIcon(large);
                let _ = DestroyIcon(small);
                println!("icon extract: ok ({:?})", t.elapsed());
            }
            Err(e) => println!("icon extract: {} ({:?})", e.code(), t.elapsed()),
        } }

        // Property handler.
        let t = Instant::now();
        let store: IPropertyStore = create(CLSID_PROPERTIES)?.cast()?;
        let stream = SHCreateStreamOnFileEx(&path, STGM_READ.0, 0, false, None)?;
        store.cast::<IInitializeWithStream>()?.Initialize(&stream, STGM_READ.0)?;
        let count = store.GetCount()?;
        println!("properties: {count} ({:?})", t.elapsed());
        for i in 0..count {
            let mut key = Default::default();
            store.GetAt(i, &mut key)?;
            let value = store.GetValue(&key)?;
            println!("   {{{:?}}} {:>2} = {value}", key.fmtid, key.pid);
        }

        // Preview handler, hosted in a window of our own.
        let t = Instant::now();
        let parent = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("shelltest"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            40,
            40,
            440,
            760,
            None,
            None,
            None,
            None,
        )?;
        let preview: IPreviewHandler = create(CLSID_PREVIEW)?.cast()?;
        preview.cast::<IInitializeWithFile>()?.Initialize(&path, STGM_READ.0)?;
        let mut rc = RECT::default();
        GetClientRect(parent, &mut rc)?;
        preview.SetWindow(parent, &rc)?;
        preview.DoPreview()?;
        pump(Duration::from_millis(600));
        let child = GetWindow(parent, GW_CHILD)?;
        capture(r, child, rc.right, rc.bottom, &save("shell_preview.png"))?;
        println!("preview: ok ({:?})", t.elapsed());

        // Resize like the preview pane does, then unload.
        let rc2 = RECT { left: 0, top: 0, right: 300, bottom: 500 };
        preview.SetRect(&rc2)?;
        pump(Duration::from_millis(300));
        capture(r, child, 300, 500, &save("shell_preview_small.png"))?;
        preview.Unload()?;
        let _ = DestroyWindow(parent);
        println!("preview unload: ok");
        Ok(())
    }
}

fn pump(duration: Duration) {
    let end = Instant::now() + duration;
    unsafe {
        let mut msg = MSG::default();
        while Instant::now() < end {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// PrintWindow with PW_RENDERFULLCONTENT (captures Direct2D output).
fn capture(r: &Renderer, hwnd: HWND, w: i32, h: i32, path: &str) -> Result<()> {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, bmp.into());
        let _ = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2));
        // PrintWindow leaves alpha at 0; make it opaque.
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        SelectObject(dc, old);
        let result = r.save_hbitmap_png(bmp, path);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        result
    }
}
