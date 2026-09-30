//! Class IDs of the handlers and the property schema.

use windows_core::GUID;

pub const CLSID_THUMBNAIL: GUID = GUID::from_u128(0x07c50bd9_7523_403d_ac91_9be7928d487a);
pub const CLSID_ICON: GUID = GUID::from_u128(0x7f6cbbfa_1bd6_4a26_a3d5_4599769ea694);
pub const CLSID_PREVIEW: GUID = GUID::from_u128(0x9f466039_090d_411e_b895_4bc797935442);
pub const CLSID_PROPERTIES: GUID = GUID::from_u128(0x1aae8d6e_7482_48a1_9b98_f7593c25292f);

/// Format ID of the `PkgPeek.*` properties (must match PkgPeek.propdesc).
pub const FMTID_PKGPEEK: GUID = GUID::from_u128(0xa484decf_d000_405f_bfbc_ea0a60607467);

/// `System.Title`.
pub const FMTID_SUMMARY_INFORMATION: GUID = GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9);

/// Registry form: `{XXXXXXXX-XXXX-...}`.
pub fn braced(guid: &GUID) -> String {
    format!("{{{guid:?}}}")
}
