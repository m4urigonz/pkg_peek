//! PKG Peek shell extension: an in-process COM server with four handlers for
//! `.pkg` files.
//!
//! | Handler            | Shows                                              |
//! |--------------------|----------------------------------------------------|
//! | Thumbnail provider | game icon + badge in medium/large views           |
//! | Icon handler       | game icon in details/list/small views             |
//! | Preview handler    | the card in Explorer's preview pane and in Peek   |
//! | Property handler   | Title ID, version... in columns, tooltips, search |
//!
//! Every entry point catches panics: this DLL runs inside explorer.exe.

mod guids;
mod icon;
mod preview;
mod props;
mod register;
mod stream;
mod thumbnail;

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, HINSTANCE, HMODULE,
    S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{DisableThreadLibraryCalls, GetModuleFileNameW};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, Ref, Result, implement};

static DLL_INSTANCE: AtomicUsize = AtomicUsize::new(0);
static OBJECTS: AtomicIsize = AtomicIsize::new(0);
static LOCKS: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn dll_instance() -> HINSTANCE {
    HINSTANCE(DLL_INSTANCE.load(Ordering::Relaxed) as *mut c_void)
}

pub(crate) fn dll_path() -> Result<PathBuf> {
    let mut buf = vec![0u16; 1024];
    let len = unsafe { GetModuleFileNameW(Some(HMODULE(dll_instance().0)), &mut buf) } as usize;
    if len == 0 || len >= buf.len() {
        return Err(windows_core::Error::from_thread());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// Keeps the DLL loaded while a COM object from it is alive.
pub(crate) struct ModuleRef;

impl ModuleRef {
    pub fn new() -> Self {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ModuleRef {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs `f`, turning a panic into `E_FAIL`.
pub(crate) fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(E_FAIL.into()))
}

pub(crate) fn io_error(e: std::io::Error) -> windows_core::Error {
    match e.raw_os_error() {
        Some(code) => HRESULT::from_win32(code as u32).into(),
        None => E_FAIL.into(),
    }
}

/// Path passed by the shell to IInitializeWithFile / IPersistFile.
pub(crate) fn path_from(p: &PCWSTR) -> Result<PathBuf> {
    if p.is_null() {
        return Err(E_POINTER.into());
    }
    Ok(PathBuf::from(unsafe { p.to_hstring() }.to_os_string()))
}

pub(crate) fn already_initialized() -> windows_core::Error {
    HRESULT::from_win32(1247).into() // ERROR_ALREADY_INITIALIZED
}

#[derive(Clone, Copy)]
enum Kind {
    Thumbnail,
    Icon,
    Preview,
    Properties,
}

#[implement(IClassFactory)]
struct ClassFactory {
    kind: Kind,
    _module: ModuleRef,
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        guard(|| {
            if ppv.is_null() {
                return Err(E_POINTER.into());
            }
            unsafe { *ppv = std::ptr::null_mut() };
            if !outer.is_null() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let object: IUnknown = match self.kind {
                Kind::Thumbnail => thumbnail::ThumbnailProvider::new().into(),
                Kind::Icon => icon::IconHandler::new().into(),
                Kind::Preview => preview::PreviewHandler::new().into(),
                Kind::Properties => props::PropertyHandler::new().into(),
            };
            unsafe { object.query(riid, ppv).ok() }
        })
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCKS.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DLL_INSTANCE.store(instance.0 as usize, Ordering::Relaxed);
        unsafe {
            let _ = DisableThreadLibraryCalls(HMODULE(instance.0));
        }
    }
    true.into()
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if clsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
        let kind = match *clsid {
            guids::CLSID_THUMBNAIL => Kind::Thumbnail,
            guids::CLSID_ICON => Kind::Icon,
            guids::CLSID_PREVIEW => Kind::Preview,
            guids::CLSID_PROPERTIES => Kind::Properties,
            _ => return CLASS_E_CLASSNOTAVAILABLE,
        };
        let factory: IClassFactory = ClassFactory { kind, _module: ModuleRef::new() }.into();
        factory.query(riid, ppv)
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 { S_OK } else { S_FALSE }
}

#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    let result = guard(|| {
        let path = dll_path()?;
        register::register(&path)
    });
    result.map_or_else(|e| e.code(), |()| S_OK)
}

#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    let result = guard(|| {
        let path = dll_path()?;
        register::unregister(&path)
    });
    result.map_or_else(|e| e.code(), |()| S_OK)
}
