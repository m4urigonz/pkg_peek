//! Read-only property store: exposes the PKG metadata to Explorer columns,
//! tooltips, the details pane, the Properties dialog and Windows Search.
//!
//! The `PkgPeek.*` properties are declared in PkgPeek.propdesc.

use std::cell::RefCell;
use std::io::BufReader;

use pkg_core::{PkgInfo, Ps4Pkg};
use windows::Win32::Foundation::{E_INVALIDARG, E_POINTER, FILETIME, PROPERTYKEY, S_FALSE, STG_E_ACCESSDENIED};
use windows::Win32::System::Com::IStream;
use windows::Win32::System::Com::StructuredStorage::{InitPropVariantFromFileTime, PROPVARIANT};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithStream, IInitializeWithStream_Impl, IPropertyStore, IPropertyStore_Impl,
    IPropertyStoreCapabilities, IPropertyStoreCapabilities_Impl,
};
use windows_core::{HRESULT, Ref, Result, implement};

use crate::stream::StreamReader;
use crate::{ModuleRef, already_initialized, guard, guids};

/// Property IDs within `guids::FMTID_PKGPEEK` (see PkgPeek.propdesc).
pub mod pid {
    pub const TITLE_ID: u32 = 2;
    pub const CONTENT_ID: u32 = 3;
    pub const CATEGORY: u32 = 4;
    pub const APP_VERSION: u32 = 5;
    pub const MASTER_VERSION: u32 = 6;
    pub const FIRMWARE: u32 = 7;
    pub const REGION: u32 = 8;
    pub const CREATION_DATE: u32 = 9;
}

const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY { fmtid: guids::FMTID_SUMMARY_INFORMATION, pid: 2 };

fn key(pid: u32) -> PROPERTYKEY {
    PROPERTYKEY { fmtid: guids::FMTID_PKGPEEK, pid }
}

#[implement(IPropertyStore, IPropertyStoreCapabilities, IInitializeWithStream)]
pub struct PropertyHandler {
    values: RefCell<Option<Vec<(PROPERTYKEY, PROPVARIANT)>>>,
    _module: ModuleRef,
}

impl PropertyHandler {
    pub fn new() -> Self {
        Self { values: RefCell::new(None), _module: ModuleRef::new() }
    }
}

fn same_key(a: &PROPERTYKEY, b: &PROPERTYKEY) -> bool {
    a.fmtid == b.fmtid && a.pid == b.pid
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "YYYY-MM-DD" at 12:00 UTC (noon keeps the date stable in any time zone).
fn date_filetime(iso: &str) -> Option<FILETIME> {
    let mut parts = iso.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let days = days_from_civil(y, m, d) - days_from_civil(1601, 1, 1);
    let ticks = ((days * 86_400 + 43_200) as u64).checked_mul(10_000_000)?;
    Some(FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 })
}

fn properties(info: &PkgInfo) -> Vec<(PROPERTYKEY, PROPVARIANT)> {
    let languages = pkg_render::ui_languages();
    let mut v = vec![
        (PKEY_TITLE, PROPVARIANT::from(info.title_for(&languages))),
        (key(pid::TITLE_ID), PROPVARIANT::from(info.title_id.as_str())),
        (key(pid::CONTENT_ID), PROPVARIANT::from(info.content_id.as_str())),
        (key(pid::CATEGORY), PROPVARIANT::from(info.category.label())),
    ];
    let mut push = |pid: u32, value: Option<String>| {
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            v.push((key(pid), PROPVARIANT::from(value.as_str())));
        }
    };
    push(pid::APP_VERSION, info.app_ver.clone());
    push(pid::MASTER_VERSION, info.version.clone());
    push(pid::FIRMWARE, info.firmware());
    push(pid::REGION, info.region.map(|r| r.label().to_string()));
    if let Some(ft) = info.creation_date.as_deref().and_then(date_filetime) {
        if let Ok(pv) = unsafe { InitPropVariantFromFileTime(&ft) } {
            v.push((key(pid::CREATION_DATE), pv));
        }
    }
    v
}

impl IInitializeWithStream_Impl for PropertyHandler_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        guard(|| {
            if self.values.borrow().is_some() {
                return Err(already_initialized());
            }
            let stream = stream.ok()?.clone();
            let reader = BufReader::with_capacity(16 * 1024, StreamReader(stream));
            // Anything that is not a readable PS4 PKG simply has no extra
            // properties; Explorer still shows the file system ones.
            let values = Ps4Pkg::open(reader)
                .ok()
                .and_then(|mut pkg| PkgInfo::read(&mut pkg).ok())
                .map(|info| properties(&info))
                .unwrap_or_default();
            *self.values.borrow_mut() = Some(values);
            Ok(())
        })
    }
}

impl IPropertyStore_Impl for PropertyHandler_Impl {
    fn GetCount(&self) -> Result<u32> {
        Ok(self.values.borrow().as_ref().map_or(0, |v| v.len() as u32))
    }

    fn GetAt(&self, index: u32, pkey: *mut PROPERTYKEY) -> Result<()> {
        if pkey.is_null() {
            return Err(E_POINTER.into());
        }
        let values = self.values.borrow();
        let (k, _) = values.as_ref().and_then(|v| v.get(index as usize)).ok_or(E_INVALIDARG)?;
        unsafe { *pkey = *k };
        Ok(())
    }

    fn GetValue(&self, pkey: *const PROPERTYKEY) -> Result<PROPVARIANT> {
        if pkey.is_null() {
            return Err(E_POINTER.into());
        }
        let wanted = unsafe { &*pkey };
        let values = self.values.borrow();
        Ok(values
            .as_ref()
            .and_then(|v| v.iter().find(|(k, _)| same_key(k, wanted)))
            .map(|(_, pv)| pv.clone())
            .unwrap_or_default())
    }

    fn SetValue(&self, _key: *const PROPERTYKEY, _value: *const PROPVARIANT) -> Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }

    fn Commit(&self) -> Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }
}

impl IPropertyStoreCapabilities_Impl for PropertyHandler_Impl {
    fn IsPropertyWritable(&self, _key: *const PROPERTYKEY) -> HRESULT {
        S_FALSE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_of_date() {
        // 2024-05-10 12:00 UTC.
        let ft = date_filetime("2024-05-10").unwrap();
        let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        let unix = ticks / 10_000_000 - 11_644_473_600;
        assert_eq!(unix, 1_715_342_400);
        assert!(date_filetime("garbage").is_none());
    }
}
