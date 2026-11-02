//! Windows specifics: HID access (hid.dll), cross-process lock, data folder,
//! language and start with Windows.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
    CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid, HidD_GetInputReport,
    HidD_GetPreparsedData, HidD_SetFeature, HidD_SetOutputReport, HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, ReleaseMutex, WaitForSingleObject};

/// Handle to the control box's vendor-defined HID collection.
pub struct Transport {
    handle: isize,
    /// Device path, to open the second read handle
    path: Vec<u16>,
    /// Overlapped handle to read the interrupt endpoint with a timeout (open_reader)
    reader: OnceLock<isize>,
}

// The handle is only used from the thread that owns the Transport.
unsafe impl Send for Transport {}

pub struct Opened {
    pub transport: Transport,
    pub in_len: usize,
    pub out_len: usize,
    pub feature_len: usize,
    /// By ID: Windows only gives the largest one in the collection (feature_len).
    pub feature_lens: Vec<(u8, usize)>,
    pub firmware: u16,
}

fn interface_paths() -> Vec<Vec<u16>> {
    unsafe {
        let guid = HidD_GetHidGuid();
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
            != CR_SUCCESS
        {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&guid, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
            != CR_SUCCESS
        {
            return Vec::new();
        }
        buf.split(|&c| c == 0)
            .filter(|p| !p.is_empty())
            .map(|p| p.iter().copied().chain(std::iter::once(0)).collect())
            .collect()
    }
}

impl Transport {
    /// Opens the vendor-defined collection (usage page 0xFF00 or higher, or exactly `usage_page` if
    /// nonzero; number `collection` if nonzero) of interface `interface` of the USB device
    /// `vid:pid` that has output reports, or feature reports if the model uses those (`feature`).
    pub fn open(vid: u16, pid: u16, interface: u8, usage_page: u16, collection: u8, feature: bool) -> Option<Opened> {
        let ids = format!("vid_{vid:04x}&pid_{pid:04x}");
        let mi = format!("&mi_{interface:02x}");
        let col = format!("&col{collection:02x}");
        for path in interface_paths() {
            let lower = String::from_utf16_lossy(&path).to_lowercase();
            let single_interface = interface == 0 && !lower.contains("&mi_");
            if !lower.contains(&ids) || !(lower.contains(&mi) || single_interface) {
                continue;
            }
            if collection != 0 && !lower.contains(&col) {
                continue;
            }
            unsafe {
                let Ok(h) = CreateFileW(
                    PCWSTR(path.as_ptr()),
                    0xC000_0000, // GENERIC_READ | GENERIC_WRITE
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                ) else {
                    continue;
                };
                let mut pp = PHIDP_PREPARSED_DATA::default();
                let mut caps = HIDP_CAPS::default();
                if HidD_GetPreparsedData(h, &mut pp) {
                    let _ = HidP_GetCaps(pp, &mut caps);
                    let _ = HidD_FreePreparsedData(pp);
                }
                let page_ok = if usage_page != 0 { caps.UsagePage == usage_page } else { caps.UsagePage >= 0xFF00 };
                let reports = if feature { caps.FeatureReportByteLength } else { caps.OutputReportByteLength };
                if page_ok && reports > 0 {
                    let mut attrs =
                        HIDD_ATTRIBUTES { Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
                    let _ = HidD_GetAttributes(h, &mut attrs);
                    return Some(Opened {
                        transport: Transport { handle: h.0 as isize, path, reader: OnceLock::new() },
                        in_len: caps.InputReportByteLength as usize,
                        out_len: caps.OutputReportByteLength as usize,
                        feature_len: caps.FeatureReportByteLength as usize,
                        feature_lens: Vec::new(),
                        firmware: attrs.VersionNumber,
                    });
                }
                let _ = CloseHandle(h);
            }
        }
        None
    }

    fn h(&self) -> HANDLE {
        HANDLE(self.handle as *mut c_void)
    }

    /// Sends an output report (with the report ID in byte 0).
    pub fn write(&self, p: &[u8]) -> bool {
        unsafe {
            let mut n = 0u32;
            WriteFile(self.h(), Some(p), Some(&mut n), None).is_ok()
                || HidD_SetOutputReport(self.h(), p.as_ptr() as *const c_void, p.len() as u32)
        }
    }

    /// GET_REPORT of the input report whose ID is in buf[0].
    pub fn get_input(&self, buf: &mut [u8]) -> bool {
        unsafe { HidD_GetInputReport(self.h(), buf.as_mut_ptr() as *mut c_void, buf.len() as u32) }
    }

    /// SET_REPORT of a feature report (with the report ID in byte 0).
    pub fn set_feature(&self, p: &[u8]) -> bool {
        unsafe { HidD_SetFeature(self.h(), p.as_ptr() as *const c_void, p.len() as u32) }
    }

    /// GET_REPORT of the feature report whose ID is in buf[0].
    pub fn get_feature(&self, buf: &mut [u8]) -> bool {
        unsafe { HidD_GetFeature(self.h(), buf.as_mut_ptr() as *mut c_void, buf.len() as u32) }
    }

    /// Waits for the next input report from the interrupt endpoint (control box buttons).
    pub fn read_report(&self, buf: &mut [u8]) -> bool {
        let mut n = 0u32;
        unsafe { ReadFile(self.h(), Some(buf), Some(&mut n), None).is_ok() }
    }

    /// For models that reply on the interrupt endpoint: opens a second, overlapped handle
    /// that receives its own copy of the reports from this moment on (before the first
    /// command, so the reply isn't lost). The main handle stays as is.
    pub fn open_reader(&self) -> bool {
        if self.reader.get().is_some() {
            return true;
        }
        let h = unsafe {
            CreateFileW(
                PCWSTR(self.path.as_ptr()),
                0xC000_0000, // GENERIC_READ | GENERIC_WRITE
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        };
        h.is_ok_and(|h| self.reader.set(h.0 as isize).is_ok())
    }

    /// Next input report from the interrupt endpoint, waiting at most `timeout`.
    /// Returns the bytes read, or None if nothing arrives (or the reader wasn't opened).
    pub fn read_timeout(&self, buf: &mut [u8], timeout: Duration) -> Option<usize> {
        let h = HANDLE(*self.reader.get()? as *mut c_void);
        unsafe {
            let event = CreateEventW(None, true, false, PCWSTR::null()).ok()?;
            let mut ov = OVERLAPPED { hEvent: event, ..Default::default() };
            let started = match ReadFile(h, Some(buf), None, Some(&mut ov)) {
                Ok(()) => true,
                Err(e) => e.code() == ERROR_IO_PENDING.to_hresult(),
            };
            let done = started && WaitForSingleObject(event, timeout.as_millis() as u32) == WAIT_OBJECT_0;
            if started && !done {
                let _ = CancelIoEx(h, Some(&ov));
            }
            let mut n = 0u32;
            // With the read cancelled or finished, waiting here doesn't block
            let ok = started && GetOverlappedResult(h, &ov, &mut n, true).is_ok();
            let _ = CloseHandle(event);
            (done && ok).then_some(n as usize)
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.h());
            if let Some(&reader) = self.reader.get() {
                let _ = CloseHandle(HANDLE(reader as *mut c_void));
            }
        }
    }
}

/// Serializes commands across processes with a named mutex.
pub struct CommandLock;

fn mutex() -> HANDLE {
    static LOCK: OnceLock<isize> = OnceLock::new();
    HANDLE(*LOCK.get_or_init(|| unsafe {
        CreateMutexW(None, false, w!("Local\\Nogenuity-HID")).map(|h| h.0 as isize).unwrap_or(0)
    }) as *mut c_void)
}

impl CommandLock {
    pub fn take() -> Option<CommandLock> {
        let r = unsafe { WaitForSingleObject(mutex(), 5000) };
        (r.0 == 0 || r.0 == 0x80).then_some(CommandLock) // WAIT_OBJECT_0 / WAIT_ABANDONED
    }
}

impl Drop for CommandLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(mutex());
        }
    }
}

/// %APPDATA%\Nogenuity
/// Opens a URL, a file or a folder with the default program.
pub fn open(target: &std::path::Path) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(target.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// On Windows no permission is needed for the headset.
pub fn begin_scan() {}

pub fn data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Nogenuity"))
}

pub fn is_spanish() -> bool {
    const LANG_SPANISH: u16 = 0x0A;
    (unsafe { GetUserDefaultUILanguage() } & 0x3FF) == LANG_SPANISH
}

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Nogenuity");
/// State of each Run entry in Task Manager. Without a value here, Windows may skip
/// the entry at sign-in, so it is written together with the Run one.
const APPROVED_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
/// What Task Manager writes when enabling (when disabling, 03 and the date)
const APPROVED_ON: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Command that starts the app hidden in the tray.
fn startup_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --tray", exe.display()))
}

/// Saved start-with-Windows value, if any.
fn run_value() -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    ok.is_ok().then(|| {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        String::from_utf16_lossy(&buf[..len])
    })
}

/// State saved in StartupApproved, if any.
fn approved_value() -> Option<Vec<u8>> {
    let mut buf = [0u8; 64];
    let mut size = buf.len() as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, APPROVED_KEY, RUN_VALUE, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    ok.is_ok().then(|| buf[..size as usize].to_vec())
}

/// Odd first byte (03, 07): disabled in Task Manager; even (02, 06): enabled.
fn approval_disabled(value: &[u8]) -> bool {
    value.first().is_some_and(|b| b & 1 == 1)
}

pub fn autostart_enabled() -> bool {
    run_value().is_some_and(|v| Some(v) == startup_command())
        && !approved_value().is_some_and(|v| approval_disabled(&v))
}

/// If the app starts with Windows but the value points to another executable, points it to this
/// one: always with `force`, otherwise only when that one no longer exists (moved or deleted). If
/// the StartupApproved state is missing (older versions didn't write it), marks it as enabled;
/// if the user disabled it in Task Manager, that is respected.
pub fn refresh_autostart(force: bool) {
    let (Some(value), Ok(exe)) = (run_value(), std::env::current_exe()) else { return };
    let target = PathBuf::from(value.trim_start_matches('"').split('"').next().unwrap_or_default());
    if target != exe && (force || !target.exists()) {
        write_run_value();
    }
    if approved_value().is_none() {
        write_approved();
    }
}

fn write_run_value() {
    let Some(cmd) = startup_command() else { return };
    let value = HSTRING::from(cmd);
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(value.as_ptr().cast()),
            ((value.len() + 1) * 2) as u32,
        );
    }
}

fn write_approved() {
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            RUN_VALUE,
            REG_BINARY.0,
            Some(APPROVED_ON.as_ptr().cast()),
            APPROVED_ON.len() as u32,
        );
    }
}

pub fn set_autostart(on: bool) {
    if on {
        write_run_value();
        write_approved();
    } else {
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, RUN_VALUE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_state_follows_task_manager() {
        assert!(!approval_disabled(&APPROVED_ON));
        assert!(!approval_disabled(&[6, 0, 0, 0]));
        assert!(approval_disabled(&[3, 0, 0, 0, 0x5a, 0x1c, 0x2b, 0x9e, 0x4f, 0x4a, 0xdb, 0x01]));
        assert!(approval_disabled(&[7]));
        assert!(!approval_disabled(&[]));
    }
}
