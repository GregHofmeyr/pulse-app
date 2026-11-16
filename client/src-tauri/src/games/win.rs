//! Windows: where game detection reads (#380). Steam's `RunningAppID` and `SteamPath` from
//! `HKCU\Software\Valve\Steam` (two small registry reads), and the process list the stream picker already uses
//! (`stream::procs`). Nothing here decides anything: that's `detect.rs`, tested on every platform.

use std::path::PathBuf;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::{PCWSTR, w};

use super::detect::Source;

const STEAM_KEY: PCWSTR = w!("Software\\Valve\\Steam");

pub struct Windows;

impl Source for Windows {
    fn running_app_id(&mut self) -> Option<u32> {
        let mut value: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: the key and value names are static wide strings; `value` is a u32 and `size` says so.
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                STEAM_KEY,
                w!("RunningAppID"),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut value as *mut u32).cast()),
                Some(&mut size),
            )
        };
        (r == ERROR_SUCCESS).then_some(value)
    }

    fn steam_path(&mut self) -> Option<PathBuf> {
        // MAX_PATH-ish paths; a longer one is read with the size Windows asks for
        let mut buf = vec![0u16; 520];
        for _ in 0..2 {
            let mut size = (buf.len() * 2) as u32;
            // SAFETY: as above; `buf` holds `size` bytes.
            let r = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    STEAM_KEY,
                    w!("SteamPath"),
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr().cast()),
                    Some(&mut size),
                )
            };
            if r == ERROR_SUCCESS {
                let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                let s = String::from_utf16_lossy(&buf[..n]);
                return (!s.trim().is_empty()).then(|| PathBuf::from(s));
            }
            // too small: grow to what it said and try once more
            buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        }
        None
    }

    fn processes(&mut self) -> Vec<String> {
        crate::stream::procs::processes()
            .into_iter()
            .map(|p| p.exe)
            .collect()
    }
}
