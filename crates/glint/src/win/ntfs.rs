//! Reads every file and folder name straight from the NTFS master file table
//! (seconds for millions of entries), then follows the USN change journal so
//! the index stays live. The journal read blocks inside the kernel until
//! something changes, so an idle machine costs no CPU.
//!
//! Requires administrator rights; without them [`index_volume`] fails and the
//! caller falls back to a plain directory walk.

use super::{pcwstr, wide};
use glint_core::files::{FileIndex, NO_PARENT};
use std::sync::RwLock;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    CREATE_USN_JOURNAL_DATA, FSCTL_CREATE_USN_JOURNAL, FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL,
    FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V0, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
};
use windows::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const DRIVE_FIXED: u32 = 3;
const ROOT_RECORD: u32 = 5;
const NONE: u32 = u32::MAX;

const REASON_FILE_CREATE: u32 = 0x0000_0100;
const REASON_FILE_DELETE: u32 = 0x0000_0200;
const REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;
const REASON_CLOSE: u32 = 0x8000_0000;

/// Fixed NTFS drives, as "C:".
pub fn ntfs_drives() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:", (b'A' + i as u8) as char))
        .filter(|d| {
            let root = wide(&format!("{d}\\"));
            let mut fs = [0u16; 16];
            unsafe {
                GetDriveTypeW(pcwstr(&root)) == DRIVE_FIXED
                    && GetVolumeInformationW(pcwstr(&root), None, None, None, None, Some(&mut fs)).is_ok()
            }
            .then(|| String::from_utf16_lossy(&fs).trim_end_matches('\0') == "NTFS")
            .unwrap_or(false)
        })
        .collect()
}

pub struct Volume {
    handle: HANDLE,
    journal_id: u64,
    next_usn: i64,
    /// MFT record number -> index id.
    ids: Vec<u32>,
}

// The handle is only used from the watcher thread after indexing.
unsafe impl Send for Volume {}

impl Drop for Volume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn record_number(frn: u64) -> u32 {
    // Low 48 bits are the record number; volumes never get near 2^32 records.
    (frn & 0xFFFF_FFFF_FFFF) as u32
}

unsafe fn ioctl<I>(h: HANDLE, code: u32, input: Option<&I>, out: &mut [u8]) -> windows::core::Result<u32> {
    let mut returned = 0u32;
    DeviceIoControl(
        h,
        code,
        input.map(|i| i as *const I as *const _),
        input.map_or(0, |_| std::mem::size_of::<I>() as u32),
        Some(out.as_mut_ptr() as *mut _),
        out.len() as u32,
        Some(&mut returned),
        None,
    )?;
    Ok(returned)
}

/// A USN_RECORD_V2 view over raw bytes.
struct Record<'a>(&'a [u8]);

impl<'a> Record<'a> {
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes(self.0[at..at + 2].try_into().unwrap())
    }
    fn u32(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.0[at..at + 4].try_into().unwrap())
    }
    fn u64(&self, at: usize) -> u64 {
        u64::from_le_bytes(self.0[at..at + 8].try_into().unwrap())
    }
    fn major(&self) -> u16 {
        self.u16(4)
    }
    fn frn(&self) -> u64 {
        self.u64(8)
    }
    fn parent(&self) -> u64 {
        self.u64(16)
    }
    fn reason(&self) -> u32 {
        self.u32(40)
    }
    fn is_dir(&self) -> bool {
        self.u32(52) & FILE_ATTRIBUTE_DIRECTORY.0 != 0
    }
    fn name(&self) -> String {
        let (len, off) = (self.u16(56) as usize, self.u16(58) as usize);
        let units: Vec<u16> = self.0[off..off + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    }
}

/// Calls `f` for each V2 record in a buffer that starts with an 8-byte header.
fn each_record(buf: &[u8], mut f: impl FnMut(Record)) {
    let mut at = 8;
    while at + 60 <= buf.len() {
        let len = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        if len == 0 || at + len > buf.len() {
            break;
        }
        let r = Record(&buf[at..at + len]);
        if r.major() == 2 {
            f(r);
        }
        at += len;
    }
}

/// Adds every name on `drive` (like "C:") to `index` and returns the volume
/// for [`watch`]. Fails without administrator rights.
pub fn index_volume(drive: &str, index: &mut FileIndex) -> windows::core::Result<Volume> {
    unsafe {
        let path = wide(&format!("\\\\.\\{drive}"));
        let handle = CreateFileW(
            pcwstr(&path),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?;
        let mut vol = Volume { handle, journal_id: 0, next_usn: 0, ids: Vec::new() };

        let mut jd = [0u8; std::mem::size_of::<USN_JOURNAL_DATA_V0>()];
        if ioctl::<()>(handle, FSCTL_QUERY_USN_JOURNAL, None, &mut jd).is_err() {
            let create = CREATE_USN_JOURNAL_DATA { MaximumSize: 32 << 20, AllocationDelta: 8 << 20 };
            ioctl(handle, FSCTL_CREATE_USN_JOURNAL, Some(&create), &mut [])?;
            ioctl::<()>(handle, FSCTL_QUERY_USN_JOURNAL, None, &mut jd)?;
        }
        let journal: USN_JOURNAL_DATA_V0 = std::ptr::read_unaligned(jd.as_ptr() as *const _);
        vol.journal_id = journal.UsnJournalID;
        vol.next_usn = journal.NextUsn;

        let root = index.push(drive, NO_PARENT, true);
        let first = index.len();
        // Parent record numbers, resolved to ids once every record is known.
        let mut parent_recs: Vec<u32> = Vec::new();
        let set_id = |ids: &mut Vec<u32>, rec: u32, id: u32| {
            if ids.len() <= rec as usize {
                ids.resize(rec as usize + 1, NONE);
            }
            ids[rec as usize] = id;
        };
        set_id(&mut vol.ids, ROOT_RECORD, root);

        let mut med = MFT_ENUM_DATA_V0 { StartFileReferenceNumber: 0, LowUsn: 0, HighUsn: journal.NextUsn };
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = match ioctl(handle, FSCTL_ENUM_USN_DATA, Some(&med), &mut buf) {
                Ok(n) => n as usize,
                Err(_) => break, // ERROR_HANDLE_EOF: done
            };
            if n <= 8 {
                break;
            }
            each_record(&buf[..n], |r| {
                let rec = record_number(r.frn());
                if rec == ROOT_RECORD {
                    return;
                }
                let name = r.name();
                // NTFS metadata files ($MFT, $Extend, ...).
                if rec < 24 && name.starts_with('$') {
                    return;
                }
                let id = index.push(&name, NO_PARENT, r.is_dir());
                set_id(&mut vol.ids, rec, id);
                parent_recs.push(record_number(r.parent()));
            });
            med.StartFileReferenceNumber = u64::from_le_bytes(buf[..8].try_into().unwrap());
        }
        for (i, prec) in parent_recs.iter().enumerate() {
            let parent = vol.ids.get(*prec as usize).copied().unwrap_or(NONE);
            index.set_parent((first + i) as u32, if parent == NONE { root } else { parent });
        }
        Ok(vol)
    }
}

/// Applies journal changes to `index` forever. Blocks in the kernel while idle.
pub fn watch(mut vol: Volume, index: &RwLock<FileIndex>) {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let req = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: vol.next_usn,
            ReasonMask: REASON_FILE_CREATE | REASON_FILE_DELETE | REASON_RENAME_NEW_NAME | REASON_CLOSE,
            ReturnOnlyOnClose: 1,
            Timeout: 0,
            BytesToWaitFor: 1,
            UsnJournalID: vol.journal_id,
        };
        let n = match unsafe { ioctl(vol.handle, FSCTL_READ_USN_JOURNAL, Some(&req), &mut buf) } {
            Ok(n) => n as usize,
            Err(_) => return, // journal deleted or volume gone; index stays as is
        };
        if n < 8 {
            continue;
        }
        vol.next_usn = i64::from_le_bytes(buf[..8].try_into().unwrap());
        let mut ix = index.write().unwrap();
        each_record(&buf[..n], |r| apply(&mut vol, &mut ix, &r));
    }
}

fn apply(vol: &mut Volume, ix: &mut FileIndex, r: &Record) {
    let reason = r.reason();
    if reason & (REASON_FILE_DELETE | REASON_FILE_CREATE | REASON_RENAME_NEW_NAME) == 0 {
        return;
    }
    let rec = record_number(r.frn()) as usize;
    let existing = vol.ids.get(rec).copied().unwrap_or(NONE);
    if reason & REASON_FILE_DELETE != 0 {
        if existing != NONE {
            ix.remove(existing);
            vol.ids[rec] = NONE;
        }
        return;
    }
    let parent = vol.ids.get(record_number(r.parent()) as usize).copied().unwrap_or(NONE);
    if parent == NONE {
        return;
    }
    if existing != NONE {
        // Renamed or moved: update in place so children follow.
        ix.rename(existing, &r.name(), parent);
        return;
    }
    let id = ix.push(&r.name(), parent, r.is_dir());
    if vol.ids.len() <= rec {
        vol.ids.resize(rec + 1, NONE);
    }
    vol.ids[rec] = id;
}
