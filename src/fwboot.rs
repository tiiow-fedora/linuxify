//! Keeps UEFI boot entries pointing at the right partition after `--undo`.
//!
//! Deleting partitions from Windows packs the GPT: the partitions after them
//! move to lower slots. A firmware boot entry stores its partition as
//! HD(number, GPT, partition-GUID, …) and most firmware (EDK2-based, which is
//! nearly all) matches on the number too, so a Linux installed behind LNXINST
//! stopped starting after `--undo` (found with Omarchy in a VM: "failed to
//! load Boot0006 … Not Found", the PC fell through to Windows). The GUID
//! still identifies the partition, so the number is looked up again and
//! rewritten. `bcdedit /set … device` was tried first: it reports success
//! but leaves the firmware's copy unchanged.

/// A GPT partition: (1-based slot, unique partition GUID as stored on disk).
pub type GptEntry = (u32, [u8; 16]);

pub fn parse_gpt_entries(buf: &[u8], count: u32, size: u32) -> Vec<GptEntry> {
    let mut out = Vec::new();
    for i in 0..count as usize {
        let o = i * size as usize;
        let Some(e) = buf.get(o..o + 32) else { break };
        if e[..16].iter().all(|&b| b == 0) {
            continue;
        }
        let mut g = [0u8; 16];
        g.copy_from_slice(&e[16..32]);
        out.push((i as u32 + 1, g));
    }
    out
}

/// The description of an EFI_LOAD_OPTION (attributes u32, path length u16,
/// UTF-16 description, device path, optional data).
pub fn load_option_description(opt: &[u8]) -> String {
    let units: Vec<u16> = opt.get(6..).unwrap_or(&[]).as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

/// A rewritten load option and each (old, new) partition number.
pub type Fixed = (Vec<u8>, Vec<(u32, u32)>);

/// Rewrites stale partition numbers in a load option's HD() nodes. Returns
/// the new option and each (old, new) number, or None when nothing changed.
/// Only nodes whose GUID is one of `parts` (this disk) are touched.
pub fn fix_load_option(opt: &[u8], parts: &[GptEntry]) -> Option<Fixed> {
    let path_len = u16::from_le_bytes([*opt.get(4)?, *opt.get(5)?]) as usize;
    let mut i = 6;
    while u16::from_le_bytes([*opt.get(i)?, *opt.get(i + 1)?]) != 0 {
        i += 2;
    }
    let start = i + 2;
    let end = start + path_len;
    if end > opt.len() {
        return None;
    }
    let mut out = opt.to_vec();
    let mut changes = Vec::new();
    let mut i = start;
    while i + 4 <= end {
        let (kind, sub, len) = (opt[i], opt[i + 1], u16::from_le_bytes([opt[i + 2], opt[i + 3]]) as usize);
        if len < 4 || i + len > end || (kind == 0x7f && sub == 0xff) {
            break;
        }
        // Media / hard drive node, GPT signature.
        if kind == 0x04 && sub == 0x01 && len == 42 && opt[i + 41] == 0x02 {
            let number = u32::from_le_bytes(opt[i + 4..i + 8].try_into().ok()?);
            if let Some(&(slot, _)) = parts.iter().find(|(_, g)| g[..] == opt[i + 24..i + 40]) {
                if slot != number {
                    out[i + 4..i + 8].copy_from_slice(&slot.to_le_bytes());
                    changes.push((number, slot));
                }
            }
        }
        i += len;
    }
    (!changes.is_empty()).then_some((out, changes))
}

/// After partitions were deleted: find (and, with `write`, fix) every
/// firmware boot entry whose partition number no longer matches its GUID on
/// any of `disks`. Disks without a readable GPT are skipped. Returns what
/// was (or would be) changed.
#[cfg(windows)]
pub fn refresh_boot_entries(disks: &[u32], write: bool) -> Result<Vec<String>, String> {
    let parts: Vec<GptEntry> = disks.iter().filter_map(|&d| sys::read_gpt(d).ok()).flatten().collect();
    if parts.is_empty() {
        return Err("could not read any disk's partition table".into());
    }
    sys::enable_privilege()?;
    let order = sys::get_var("BootOrder").ok_or("could not read the firmware's BootOrder")?.0;
    let mut done = Vec::new();
    for n in order.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)) {
        let name = format!("Boot{n:04X}");
        let Some((opt, attrs)) = sys::get_var(&name) else { continue };
        if let Some((fixed, changes)) = fix_load_option(&opt, &parts) {
            if write {
                sys::set_var(&name, &fixed, attrs)?;
            }
            for (old, new) in changes {
                done.push(format!("{name} \"{}\": partition {old} -> {new}", load_option_description(&opt)));
            }
        }
    }
    Ok(done)
}

#[cfg(not(windows))]
pub fn refresh_boot_entries(_disks: &[u32], _write: bool) -> Result<Vec<String>, String> {
    Err("only possible on Windows".into())
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::io::{Read, Seek, SeekFrom};

    #[repr(C)]
    struct Luid {
        low: u32,
        high: i32,
    }
    #[repr(C)]
    struct TokenPrivileges {
        count: u32,
        luid: Luid,
        attrs: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFirmwareEnvironmentVariableExW(name: *const u16, guid: *const u16, buf: *mut c_void, size: u32, attrs: *mut u32) -> u32;
        fn SetFirmwareEnvironmentVariableExW(name: *const u16, guid: *const u16, buf: *const c_void, size: u32, attrs: u32) -> i32;
        fn GetCurrentProcess() -> isize;
        fn CloseHandle(h: isize) -> i32;
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
        fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut Luid) -> i32;
        fn AdjustTokenPrivileges(token: isize, disable_all: i32, new: *const TokenPrivileges, len: u32, prev: *mut c_void, ret: *mut u32) -> i32;
    }
    const EFI_GLOBAL: &str = "{8BE4DF61-93CA-11D2-AA0D-00E098032B8C}";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Writing firmware variables needs SeSystemEnvironmentPrivilege enabled
    /// (administrators hold it, disabled by default).
    pub fn enable_privilege() -> Result<(), String> {
        unsafe {
            let mut token = 0isize;
            if OpenProcessToken(GetCurrentProcess(), 0x20 | 0x08, &mut token) == 0 {
                return Err("could not open the process token".into());
            }
            let mut luid = Luid { low: 0, high: 0 };
            let ok = LookupPrivilegeValueW(std::ptr::null(), wide("SeSystemEnvironmentPrivilege").as_ptr(), &mut luid) != 0
                && AdjustTokenPrivileges(token, 0, &TokenPrivileges { count: 1, luid, attrs: 2 }, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0;
            CloseHandle(token);
            if ok { Ok(()) } else { Err(format!("could not enable SeSystemEnvironmentPrivilege: {}", std::io::Error::last_os_error())) }
        }
    }

    pub fn get_var(name: &str) -> Option<(Vec<u8>, u32)> {
        let mut buf = vec![0u8; 64 * 1024];
        let mut attrs = 0u32;
        let n = unsafe { GetFirmwareEnvironmentVariableExW(wide(name).as_ptr(), wide(EFI_GLOBAL).as_ptr(), buf.as_mut_ptr().cast(), buf.len() as u32, &mut attrs) };
        (n > 0).then(|| {
            buf.truncate(n as usize);
            (buf, attrs)
        })
    }

    pub fn set_var(name: &str, data: &[u8], attrs: u32) -> Result<(), String> {
        let ok = unsafe { SetFirmwareEnvironmentVariableExW(wide(name).as_ptr(), wide(EFI_GLOBAL).as_ptr(), data.as_ptr().cast(), data.len() as u32, attrs) };
        if ok != 0 { Ok(()) } else { Err(format!("could not write firmware variable {name}: {}", std::io::Error::last_os_error())) }
    }

    /// Reads the primary GPT straight from the disk (512- or 4096-byte sectors).
    pub fn read_gpt(disk: u32) -> Result<Vec<super::GptEntry>, String> {
        let path = format!(r"\\.\PhysicalDrive{disk}");
        let err = |e: std::io::Error| format!("could not read {path}: {e}");
        let mut f = std::fs::File::open(&path).map_err(err)?;
        let mut head = vec![0u8; 8192];
        f.read_exact(&mut head).map_err(err)?;
        let sector = [512usize, 4096].into_iter().find(|&s| &head[s..s + 8] == b"EFI PART").ok_or(format!("{path} has no GPT header"))?;
        let h = &head[sector..];
        let lba = u64::from_le_bytes(h[72..80].try_into().unwrap_or_default());
        let count = u32::from_le_bytes(h[80..84].try_into().unwrap_or_default());
        let size = u32::from_le_bytes(h[84..88].try_into().unwrap_or_default());
        if !(128..=4096).contains(&size) || count > 1024 {
            return Err(format!("{path}: unexpected GPT layout ({count} entries of {size} bytes)"));
        }
        let len = (count as usize * size as usize).div_ceil(sector) * sector;
        let mut buf = vec![0u8; len];
        f.seek(SeekFrom::Start(lba * sector as u64)).map_err(err)?;
        f.read_exact(&mut buf).map_err(err)?;
        Ok(super::parse_gpt_entries(&buf, count, size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESP: [u8; 16] = [0x87, 0xa2, 0x7b, 0xea, 0xf4, 0x80, 0x42, 0x4f, 0x8a, 0x63, 0x52, 0xb2, 0xec, 0xdb, 0x14, 0xcc];

    /// The Omarchy VM's Limine entry: HD(7,GPT,ea7ba287-…,0x87ff800,0x400001)/\EFI\limine\limine_x64.efi
    fn limine(number: u32, optional: &[u8]) -> Vec<u8> {
        let mut path = vec![0x04, 0x01, 42, 0];
        path.extend(number.to_le_bytes());
        path.extend(0x87ff800u64.to_le_bytes());
        path.extend(0x400001u64.to_le_bytes());
        path.extend(ESP);
        path.extend([0x02, 0x02]);
        let file: Vec<u8> = r"\EFI\limine\limine_x64.efi".encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect();
        path.extend([0x04, 0x04]);
        path.extend((4 + file.len() as u16).to_le_bytes());
        path.extend(file);
        path.extend([0x7f, 0xff, 4, 0]);
        let mut o = 1u32.to_le_bytes().to_vec();
        o.extend((path.len() as u16).to_le_bytes());
        o.extend("Limine".encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()));
        o.extend(path);
        o.extend(optional);
        o
    }

    #[test]
    fn stale_partition_number_is_rewritten() {
        let parts = [(2, [1u8; 16]), (5, ESP), (6, [2u8; 16])];
        let (fixed, changes) = fix_load_option(&limine(7, b"WINDOWS"), &parts).unwrap();
        assert_eq!(changes, vec![(7, 5)]);
        assert_eq!(fixed, limine(5, b"WINDOWS"), "only the number changes; optional data kept");
        assert_eq!(load_option_description(&fixed), "Limine");
        assert_eq!(fix_load_option(&fixed, &parts), None, "already right");
        assert_eq!(fix_load_option(&limine(7, b""), &[(5, [9u8; 16])]), None, "other disk's partition: untouched");
        assert_eq!(fix_load_option(&limine(7, b"")[..20], &parts), None, "truncated: untouched");
    }

    #[test]
    fn gpt_entries_skip_empty_slots() {
        let mut buf = vec![0u8; 128 * 4];
        buf[0] = 1; // slot 1 used
        buf[16..32].copy_from_slice(&[3u8; 16]);
        buf[3 * 128] = 1; // slot 4 used
        buf[3 * 128 + 16..3 * 128 + 32].copy_from_slice(&ESP);
        assert_eq!(parse_gpt_entries(&buf, 4, 128), vec![(1, [3u8; 16]), (4, ESP)]);
    }
}
