//! Which fan headers this board actually routes, keyed by the board name in the registry.
//!
//! The NCT6687D-R has eight fan channels; a board may not route every channel to a header.
//! On the MSI PRO B850-P WIFI (MS-7E56) the read-only chip dump of issue #128 (2026-11-03)
//! showed `sys_fan5`/`sys_fan6` reporting 0% duty with their BIOS curves above 0, while
//! empty-but-present headers reported 50.2%; retailer spec sheets list six headers for the
//! board (CPU fan, one pump/system combo, four system fans), so `SYS_FAN5` and `SYS_FAN6`
//! do not exist on it. This module carries the one verified entry.

use std::collections::BTreeSet;

/// Where the baseboard name comes from, behind a trait so tests inject it. Production reads
/// `HKLM\HARDWARE\DESCRIPTION\System\BIOS` value `BaseBoardProduct` once at startup.
pub trait BoardProbe: Send {
    /// The board product string (e.g. `PRO B850-P WIFI (MS-7E56)`), or `None` when it
    /// could not be read.
    fn base_board_product(&self) -> Option<String>;
}

/// The production probe: a read of the registry value, nothing else.
pub struct RealBoardProbe;

impl BoardProbe for RealBoardProbe {
    fn base_board_product(&self) -> Option<String> {
        read_base_board_product()
    }
}

/// The board the verified entry belongs to, in normalised form (see [`normalize_product`]).
pub const MS7E56_NORMALISED: &str = "pro b850-p wifi (ms-7e56)";

/// Per-board headers the chip has but the board does not route, keyed by normalised board
/// product. One entry: the MSI PRO B850-P WIFI (MS-7E56) from issue #128 (see the module
/// doc). New entries need a claim verified against the board's manual or measured sensors.
pub const ABSENT_HEADERS: &[(&str, &[&str])] = &[(
    MS7E56_NORMALISED,
    &["sys_fan5", "sys_fan6"],
)];

/// Trims and lowercases a board product so registry reads compare equal across casing.
pub fn normalize_product(product: &str) -> String {
    product.trim().to_ascii_lowercase()
}

/// The headers absent on the board that reports `board_product`, or empty for an unknown
/// board (every header is then reported as before).
pub fn absent_headers_for(board_product: Option<&str>) -> BTreeSet<String> {
    let Some(product) = board_product else {
        return BTreeSet::new();
    };
    let normalised = normalize_product(product);
    let mut absent = BTreeSet::new();
    for (entry, headers) in ABSENT_HEADERS {
        if *entry == normalised {
            absent.extend(headers.iter().map(|h| h.to_string()));
        }
    }
    absent
}

/// Reads `BaseBoardProduct` under `HKLM\HARDWARE\DESCRIPTION\System\BIOS`, read-only. Returns
/// `None` on any failure. Called once at service startup (see `app.rs`).
#[cfg(windows)]
pub fn read_base_board_product() -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let path = wide(r"HARDWARE\DESCRIPTION\System\BIOS");
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut key)
            != ERROR_SUCCESS
        {
            return None;
        }
        let value = wide("BaseBoardProduct");
        let mut size: u32 = 0;
        let rc = RegQueryValueExW(key, value.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut size);
        if rc != ERROR_SUCCESS || size == 0 {
            let _ = RegCloseKey(key);
            return None;
        }
        let mut buffer = vec![0u16; (size as usize / 2) + 1];
        let mut written = size;
        let rc = RegQueryValueExW(
            key,
            value.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast::<u8>(),
            &mut written,
        );
        let _ = RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return None;
        }
        let text = String::from_utf16_lossy(&buffer).trim_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// Off Windows there is no registry to read.
#[cfg(not(windows))]
pub fn read_base_board_product() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_and_matches_ms7e56() {
        assert_eq!(absent_headers_for(Some("pro b850-p wifi (ms-7e56)")), BTreeSet::from(["sys_fan5".to_string(), "sys_fan6".to_string()]));
        assert_eq!(absent_headers_for(Some("  PRO B850-P WIFI (MS-7E56)  ")), absent_headers_for(Some("pro b850-p wifi (ms-7e56)")));
    }

    #[test]
    fn an_unknown_board_absents_nothing() {
        assert!(absent_headers_for(Some("Some other board")).is_empty());
        assert!(absent_headers_for(None).is_empty());
    }
}
