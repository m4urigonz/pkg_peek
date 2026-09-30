//! Registry entries written by `regsvr32` (DllRegisterServer) and removed by
//! `regsvr32 /u`. Machine-wide (HKLM): the property handler and the property
//! schema require it, so registration needs an elevated prompt.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::PropertiesSystem::{PSRegisterPropertySchema, PSUnregisterPropertySchema};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows_core::{GUID, HSTRING, Result};

use crate::guids::{self, braced};

pub const PROGID: &str = "PkgPeek.Package";
const EXT: &str = ".pkg";
const CLASSES: &str = r"Software\Classes";
const PREVIEW_HANDLERS: &str = r"Software\Microsoft\Windows\CurrentVersion\PreviewHandlers";
const PROPERTY_HANDLER: &str = r"Software\Microsoft\Windows\CurrentVersion\PropertySystem\PropertyHandlers\.pkg";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved";
/// Where a previous default ProgID of `.pkg` is kept to restore it later.
const PREVIOUS_PROGID: &str = "PkgPeek.PreviousProgID";

const SHELLEX_THUMBNAIL: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";
const SHELLEX_PREVIEW: &str = "{8895B1C6-B41F-4C1C-A562-0D564250836F}";
/// prevhost.exe surrogate for 64-bit preview handlers.
const PREVHOST_APPID: &str = "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}";

const SCHEMA_FILE: &str = "PkgPeek.propdesc";
const VIEWER_EXE: &str = "pkgpeek.exe";

const INFO_TIP: &str =
    "prop:System.Title;PkgPeek.TitleId;PkgPeek.Category;PkgPeek.AppVersion;PkgPeek.Firmware;System.Size";
const PREVIEW_TITLE: &str = "prop:System.Title;System.ItemTypeText";
const PREVIEW_DETAILS: &str = "prop:PkgPeek.TitleId;PkgPeek.Category;PkgPeek.AppVersion;PkgPeek.MasterVersion;\
    PkgPeek.Firmware;PkgPeek.Region;PkgPeek.ContentId;PkgPeek.CreationDate;System.Size;System.DateModified";
const FULL_DETAILS: &str = "prop:System.PropGroup.Description;System.Title;PkgPeek.TitleId;PkgPeek.ContentId;\
    PkgPeek.Category;PkgPeek.AppVersion;PkgPeek.MasterVersion;PkgPeek.Firmware;PkgPeek.Region;PkgPeek.CreationDate;\
    System.PropGroup.FileSystem;System.ItemNameDisplay;System.ItemTypeText;System.ItemFolderPathDisplay;System.Size;\
    System.DateCreated;System.DateModified;System.FileAttributes;System.FileOwner;System.ComputerName";
const TILE_INFO: &str = "prop:System.Title;PkgPeek.TitleId;PkgPeek.AppVersion";
const EXTENDED_TILE_INFO: &str = "prop:System.Title;PkgPeek.TitleId;PkgPeek.Category;PkgPeek.AppVersion";
const CONTENT_VIEW: &str =
    "prop:~System.ItemNameDisplay;System.Title;PkgPeek.TitleId;PkgPeek.Category;PkgPeek.AppVersion;System.Size";

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn check(err: WIN32_ERROR) -> Result<()> {
    if err == ERROR_SUCCESS { Ok(()) } else { Err(err.into()) }
}

fn create(path: &str) -> Result<Key> {
    let mut key = HKEY::default();
    unsafe {
        check(RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_READ,
            None,
            &mut key,
            None,
        ))?;
    }
    Ok(Key(key))
}

impl Key {
    fn set(&self, name: Option<&str>, value: &str) -> Result<&Self> {
        let data: Vec<u8> = value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
        let name = name.map(HSTRING::from).unwrap_or_default();
        unsafe { check(RegSetValueExW(self.0, &name, None, REG_SZ, Some(&data)))? };
        Ok(self)
    }

    fn set_dword(&self, name: &str, value: u32) -> Result<&Self> {
        unsafe { check(RegSetValueExW(self.0, &HSTRING::from(name), None, REG_DWORD, Some(&value.to_le_bytes())))? };
        Ok(self)
    }
}

fn get(path: &str, name: Option<&str>) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let name = name.map(HSTRING::from).unwrap_or_default();
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let chars = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..chars]))
}

fn delete_tree(path: &str) -> Result<()> {
    match unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(path)) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => {}
        err => return Err(err.into()),
    }
    // RegDeleteTreeW empties the key but leaves it; remove it too.
    match unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, &HSTRING::from(path)) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        err => Err(err.into()),
    }
}

fn delete_value(path: &str, name: Option<&str>) -> Result<()> {
    let name = name.map(HSTRING::from).unwrap_or_default();
    match unsafe { RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(path), &name) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        err => Err(err.into()),
    }
}

fn classes(path: &str) -> String {
    format!(r"{CLASSES}\{path}")
}

const HANDLERS: [(GUID, &str); 4] = [
    (guids::CLSID_THUMBNAIL, "PKG Peek Thumbnail Provider"),
    (guids::CLSID_ICON, "PKG Peek Icon Handler"),
    (guids::CLSID_PREVIEW, "PKG Peek Preview Handler"),
    (guids::CLSID_PROPERTIES, "PKG Peek Property Handler"),
];

pub fn register(dll: &Path) -> Result<()> {
    let dll_str = dll.to_string_lossy();
    let dir = dll.parent().unwrap_or(Path::new("."));

    for (clsid, name) in HANDLERS {
        let clsid_key = classes(&format!(r"CLSID\{}", braced(&clsid)));
        create(&clsid_key)?.set(None, name)?;
        create(&format!(r"{clsid_key}\InprocServer32"))?.set(None, &dll_str)?.set(Some("ThreadingModel"), "Apartment")?;
        create(APPROVED)?.set(Some(&braced(&clsid)), name)?;
    }
    let thumbnail = braced(&guids::CLSID_THUMBNAIL);
    let preview = braced(&guids::CLSID_PREVIEW);
    // Needed for IInitializeWithFile (path-based loading).
    create(&classes(&format!(r"CLSID\{thumbnail}")))?.set_dword("DisableProcessIsolation", 1)?;
    create(&classes(&format!(r"CLSID\{preview}")))?
        .set(Some("AppID"), PREVHOST_APPID)?
        .set(Some("DisplayName"), "PKG Peek")?;

    // .pkg -> PkgPeek.Package (remembering any previous owner).
    let ext = classes(EXT);
    if let Some(prev) = get(&ext, None).filter(|p| !p.is_empty() && p != PROGID) {
        create(&ext)?.set(Some(PREVIOUS_PROGID), &prev)?;
    }
    create(&ext)?.set(None, PROGID)?;
    create(&format!(r"{ext}\OpenWithProgids"))?.set(Some(PROGID), "")?;
    create(&format!(r"{ext}\ShellEx\{SHELLEX_THUMBNAIL}"))?.set(None, &thumbnail)?;
    create(&format!(r"{ext}\ShellEx\{SHELLEX_PREVIEW}"))?.set(None, &preview)?;

    let progid = classes(PROGID);
    create(&progid)?
        .set(None, "Paquete PS4")?
        .set(Some("FriendlyTypeName"), "Paquete PS4 (PKG)")?
        .set(Some("InfoTip"), INFO_TIP)?
        .set(Some("PreviewTitle"), PREVIEW_TITLE)?
        .set(Some("PreviewDetails"), PREVIEW_DETAILS)?
        .set(Some("FullDetails"), FULL_DETAILS)?
        .set(Some("TileInfo"), TILE_INFO)?
        .set(Some("ExtendedTileInfo"), EXTENDED_TILE_INFO)?
        .set(Some("ContentViewModeForBrowse"), CONTENT_VIEW)?
        .set(Some("ContentViewModeForSearch"), CONTENT_VIEW)?
        // No app-icon overlay and no frame on the thumbnails.
        .set(Some("TypeOverlay"), "")?
        .set_dword("Treatment", 0)?;
    create(&format!(r"{progid}\DefaultIcon"))?.set(None, "%1")?;
    create(&format!(r"{progid}\shellex\IconHandler"))?.set(None, &braced(&guids::CLSID_ICON))?;
    create(&format!(r"{progid}\ShellEx\{SHELLEX_THUMBNAIL}"))?.set(None, &thumbnail)?;
    create(&format!(r"{progid}\ShellEx\{SHELLEX_PREVIEW}"))?.set(None, &preview)?;
    let viewer = dir.join(VIEWER_EXE);
    create(&format!(r"{progid}\shell"))?.set(None, "open")?;
    create(&format!(r"{progid}\shell\open"))?.set(None, "Abrir con PKG Peek")?;
    create(&format!(r"{progid}\shell\open\command"))?.set(None, &format!("\"{}\" \"%1\"", viewer.display()))?;

    create(PREVIEW_HANDLERS)?.set(Some(&preview), "PKG Peek")?;
    create(PROPERTY_HANDLER)?.set(None, &braced(&guids::CLSID_PROPERTIES))?;

    let schema = dir.join(SCHEMA_FILE);
    if schema.exists() {
        let schema = HSTRING::from(schema.as_os_str());
        unsafe {
            // Unregister first so an updated file is re-read.
            let _ = PSUnregisterPropertySchema(&schema);
            PSRegisterPropertySchema(&schema)?;
        }
    }

    notify();
    Ok(())
}

pub fn unregister(dll: &Path) -> Result<()> {
    let dir = dll.parent().unwrap_or(Path::new("."));
    let schema = dir.join(SCHEMA_FILE);
    if schema.exists() {
        unsafe {
            let _ = PSUnregisterPropertySchema(&HSTRING::from(schema.as_os_str()));
        }
    }

    delete_tree(PROPERTY_HANDLER)?;
    delete_value(PREVIEW_HANDLERS, Some(&braced(&guids::CLSID_PREVIEW)))?;
    for (clsid, _) in HANDLERS {
        delete_tree(&classes(&format!(r"CLSID\{}", braced(&clsid))))?;
        delete_value(APPROVED, Some(&braced(&clsid)))?;
    }
    delete_tree(&classes(PROGID))?;

    let ext = classes(EXT);
    delete_tree(&format!(r"{ext}\ShellEx\{SHELLEX_THUMBNAIL}"))?;
    delete_tree(&format!(r"{ext}\ShellEx\{SHELLEX_PREVIEW}"))?;
    delete_value(&format!(r"{ext}\OpenWithProgids"), Some(PROGID))?;
    if get(&ext, None).as_deref() == Some(PROGID) {
        match get(&ext, Some(PREVIOUS_PROGID)) {
            Some(prev) => {
                create(&ext)?.set(None, &prev)?;
                delete_value(&ext, Some(PREVIOUS_PROGID))?;
            }
            None => delete_value(&ext, None)?,
        }
    }
    remove_if_empty(&format!(r"{ext}\ShellEx"));
    remove_if_empty(&format!(r"{ext}\OpenWithProgids"));
    remove_if_empty(&ext);

    notify();
    Ok(())
}

/// Deletes a key left without values or subkeys (so uninstalling leaves no trace).
fn remove_if_empty(path: &str) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(path), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return;
        }
        let key = Key(key);
        let (mut subkeys, mut values) = (0u32, 0u32);
        let err = RegQueryInfoKeyW(
            key.0,
            None,
            None,
            None,
            Some(&mut subkeys),
            None,
            None,
            Some(&mut values),
            None,
            None,
            None,
            None,
        );
        drop(key);
        if err == ERROR_SUCCESS && subkeys == 0 && values == 0 {
            let _ = RegDeleteKeyW(HKEY_LOCAL_MACHINE, &HSTRING::from(path));
        }
    }
}

fn notify() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}
