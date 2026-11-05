//! Linux specifics: HID access via hidraw, cross-process lock (flock), data
//! folder (~/.config/nogenuity), language and session startup (~/.config/autostart).

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};


/// true if, in the last headset search, it was found but couldn't be opened due to permissions
/// (missing udev rule).
static PERMISSION_DENIED: AtomicBool = AtomicBool::new(false);

pub fn needs_udev_rule() -> bool {
    PERMISSION_DENIED.load(Ordering::Relaxed)
}

/// Starts a headset search: if the one that couldn't be opened is gone (unplugged), stop
/// asking for permission.
pub fn begin_scan() {
    PERMISSION_DENIED.store(false, Ordering::Relaxed);
}

/// udev rule embedded in the binary: can be installed from the standalone binary or an AppImage.
const UDEV_RULE: &str = include_str!("../packaging/linux/70-nogenuity.rules");
const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/70-nogenuity.rules";
/// The PipeWire profile that rule selects. PipeWire looks in /etc before its own folder.
const AUDIO_PATH: &str = include_str!("../packaging/linux/alsa-card-profile/paths/nogenuity-alpha-s-game.conf");
const AUDIO_PROFILE: &str = include_str!("../packaging/linux/alsa-card-profile/profile-sets/nogenuity-alpha-s.conf");
const AUDIO_DIR: &str = "/etc/alsa-card-profile/mixer";

pub enum GrantError {
    /// No pkexec (polkit) on the system.
    NoPkexec,
    /// The user closed the dialog, lacks permission, or there is no polkit agent to ask.
    Cancelled,
    /// Some command already failed as root.
    Failed,
}

/// Command run as root. Contains only fixed text (the files go in single quotes, which they don't
/// contain) and fixed paths: nothing from the user or the environment. Arch and Fedora
/// lack the plugdev group: there it is removed from the rule (udev would ignore it with a warning; uaccess is enough).
/// Any failure exits with 3, so it isn't confused with pkexec's 126/127 codes.
fn grant_script() -> String {
    let rule = UDEV_RULE.replace('\r', "");
    let (audio_path, audio_profile) = (AUDIO_PATH.replace('\r', ""), AUDIO_PROFILE.replace('\r', ""));
    format!(
        "{{ umask 022 && printf '%s' '{rule}' > {UDEV_RULE_PATH} && \
         if command -v getent > /dev/null && ! getent group plugdev > /dev/null; then \
         sed -i 's/, GROUP=\"plugdev\"//' {UDEV_RULE_PATH}; fi && \
         mkdir -p {AUDIO_DIR}/paths {AUDIO_DIR}/profile-sets && \
         printf '%s' '{audio_path}' > {AUDIO_DIR}/paths/nogenuity-alpha-s-game.conf && \
         printf '%s' '{audio_profile}' > {AUDIO_DIR}/profile-sets/nogenuity-alpha-s.conf && \
         udevadm control --reload-rules && udevadm trigger --subsystem-match=hidraw --action=change; }} || exit 3"
    )
}

/// Installs the udev rule as root with a single password prompt (pkexec) and reloads udev, so the
/// headset can be opened without unplugging it. Blocks until the user responds: call
/// from a separate thread.
pub fn grant_access() -> Result<(), GrantError> {
    let status = std::process::Command::new("pkexec")
        .args(["/bin/sh", "-c", &grant_script()])
        .status()
        .map_err(|_| GrantError::NoPkexec)?;
    match status.code() {
        Some(0) => Ok(()),
        // 126: dialog closed; 127: no permission or no polkit agent (the script exits with 3)
        Some(126 | 127) => Err(GrantError::Cancelled),
        _ => Err(GrantError::Failed),
    }
}


/// /dev/hidrawN of a headset's control interface.
pub struct Transport {
    file: File,
}

pub struct Opened {
    pub transport: Transport,
    pub in_len: usize,
    pub out_len: usize,
    pub feature_len: usize,
    /// Size of each feature report by ID, from the descriptor (hidraw doesn't report it).
    pub feature_lens: Vec<(u8, usize)>,
    pub firmware: u16,
}

/// HIDIOCGINPUT(len): GET_REPORT of an input report (kernel 5.11+).
/// hidraw ioctl `nr` (HIDIOCGINPUT = 0x0A, HIDIOCSFEATURE = 0x06, HIDIOCGFEATURE = 0x07) for a
/// buffer of `len` bytes.
fn hidioc(nr: libc::c_ulong, len: usize) -> libc::c_ulong {
    const IOC_READ_WRITE: libc::c_ulong = 3;
    (IOC_READ_WRITE << 30) | ((len as libc::c_ulong) << 16) | ((b'H' as libc::c_ulong) << 8) | nr
}

/// Size in bytes (including the ID) of each feature report in the HID descriptor, by report ID.
/// hidraw doesn't report it, and a SET_FEATURE of a different size may be rejected.
fn feature_lengths(desc: &[u8]) -> Vec<(u8, usize)> {
    let (mut size, mut count, mut id) = (0usize, 0usize, 0u8);
    // Push / Pop save and restore these three (global state)
    let mut stack: Vec<(usize, usize, u8)> = Vec::new();
    let mut bits: Vec<(u8, usize)> = Vec::new();
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: skipped
            i += 3 + desc.get(i + 1).copied().unwrap_or(0) as usize;
            continue;
        }
        let n = [0, 1, 2, 4][(prefix & 0x03) as usize];
        let data = desc.get(i + 1..i + 1 + n).unwrap_or(&[]);
        let value = data.iter().rev().fold(0usize, |v, &b| v << 8 | b as usize);
        match prefix & 0xFC {
            0x74 => size = value,         // Report Size
            0x94 => count = value,        // Report Count
            0x84 => id = value as u8,     // Report ID
            0xA4 => stack.push((size, count, id)),
            0xB4 => (size, count, id) = stack.pop().unwrap_or((size, count, id)),
            0xB0 => match bits.iter_mut().find(|(r, _)| *r == id) {
                Some((_, b)) => *b += size * count,
                None => bits.push((id, size * count)),
            },
            _ => {}
        }
        i += 1 + n;
    }
    // With report IDs byte 0 is the ID; without them hidraw sends it too (0)
    bits.into_iter().map(|(id, b)| (id, b.div_ceil(8) + 1)).collect()
}

fn find_hidraw(vid: u16, pid: u16, interface: u8) -> Option<(PathBuf, u16)> {
    let hid_id = format!("HID_ID=0003:{vid:08X}:{pid:08X}");
    let input = format!("/input{interface}");
    for entry in std::fs::read_dir("/sys/class/hidraw").ok()?.flatten() {
        let sys = entry.path();
        let uevent = std::fs::read_to_string(sys.join("device/uevent")).unwrap_or_default();
        let ours = uevent.lines().any(|l| l.eq_ignore_ascii_case(&hid_id));
        // One hidraw per USB interface: HID_PHYS ends in /input<n>
        let control = uevent.lines().any(|l| l.starts_with("HID_PHYS=") && l.ends_with(&input));
        if !(ours && control) {
            continue;
        }
        // device -> HID device; its grandparent is the USB device, which has bcdDevice
        let firmware = std::fs::canonicalize(sys.join("device"))
            .ok()
            .and_then(|p| std::fs::read_to_string(p.parent()?.parent()?.join("bcdDevice")).ok())
            .and_then(|s| u16::from_str_radix(s.trim(), 16).ok())
            .unwrap_or(0);
        return Some((PathBuf::from("/dev").join(entry.file_name()), firmware));
    }
    None
}

impl Transport {
    /// `usage_page` is unused: hidraw gives one node per interface, with all its collections.
    /// `collection` and `feature` aren't used either: the interface node serves for everything.
    pub fn open(vid: u16, pid: u16, interface: u8, _usage_page: u16, _collection: u8, _feature: bool) -> Option<Opened> {
        let (path, firmware) = find_hidraw(vid, pid, interface)?;
        let feature_lens = std::fs::canonicalize(Path::new("/sys/class/hidraw").join(path.file_name()?).join("device"))
            .ok()
            .and_then(|d| std::fs::read(d.join("report_descriptor")).ok())
            .map_or(Vec::new(), |desc| feature_lengths(&desc));
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => {
                PERMISSION_DENIED.store(false, Ordering::Relaxed);
                // hidraw doesn't report the report sizes: each model sets it (0 = unknown)
                Some(Opened { transport: Transport { file }, in_len: 0, out_len: 0, feature_len: 0, feature_lens, firmware })
            }
            Err(e) => {
                PERMISSION_DENIED.store(e.kind() == std::io::ErrorKind::PermissionDenied, Ordering::Relaxed);
                None
            }
        }
    }

    /// Sends an output report (with the report ID in byte 0).
    pub fn write(&self, p: &[u8]) -> bool {
        (&self.file).write(p).is_ok_and(|n| n == p.len())
    }

    /// GET_REPORT of the input report whose ID is in buf[0].
    pub fn get_input(&self, buf: &mut [u8]) -> bool {
        unsafe { libc::ioctl(self.file.as_raw_fd(), hidioc(0x0A, buf.len()) as _, buf.as_mut_ptr()) >= 0 }
    }

    /// SET_REPORT of a feature report (with the report ID in byte 0).
    pub fn set_feature(&self, p: &[u8]) -> bool {
        unsafe { libc::ioctl(self.file.as_raw_fd(), hidioc(0x06, p.len()) as _, p.as_ptr()) >= 0 }
    }

    /// GET_REPORT of the feature report whose ID is in buf[0].
    pub fn get_feature(&self, buf: &mut [u8]) -> bool {
        unsafe { libc::ioctl(self.file.as_raw_fd(), hidioc(0x07, buf.len()) as _, buf.as_mut_ptr()) >= 0 }
    }

    /// Waits for the next input report from the interrupt endpoint (control box buttons).
    pub fn read_report(&self, buf: &mut [u8]) -> bool {
        (&self.file).read(buf).is_ok_and(|n| n > 0)
    }

    /// On Linux each hidraw descriptor already receives its own copy of the reports.
    pub fn open_reader(&self) -> bool {
        true
    }

    /// Next input report from the interrupt endpoint, waiting at most `timeout`.
    pub fn read_timeout(&self, buf: &mut [u8], timeout: Duration) -> Option<usize> {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        if unsafe { libc::poll(&mut pfd, 1, ms) } <= 0 {
            return None;
        }
        (&self.file).read(buf).ok().filter(|&n| n > 0)
    }
}

/// Environment variable holding an absolute path; the XDG spec treats empty or relative as
/// unset.
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn runtime_dir() -> PathBuf {
    env_dir("XDG_RUNTIME_DIR").unwrap_or_else(std::env::temp_dir)
}

/// Serializes commands across processes with flock on a lock file.
pub struct CommandLock {
    file: File,
}

impl CommandLock {
    pub fn take() -> Option<CommandLock> {
        let file = OpenOptions::new().create(true).write(true).open(runtime_dir().join("nogenuity-hid.lock")).ok()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Some(CommandLock { file });
            }
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for CommandLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// Opens a URL, a file or a folder with the default program.
pub fn open(target: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

/// ~/.config/nogenuity
pub fn data_dir() -> Option<PathBuf> {
    let config = env_dir("XDG_CONFIG_HOME").or_else(|| Some(env_dir("HOME")?.join(".config")))?;
    Some(config.join("nogenuity"))
}

pub fn runtime_socket() -> PathBuf {
    runtime_dir().join("nogenuity.sock")
}

pub fn is_spanish() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .is_some_and(|v| v.starts_with("es"))
}

fn autostart_file() -> Option<PathBuf> {
    Some(data_dir()?.parent()?.join("autostart").join("nogenuity.desktop"))
}

/// Quoted path for the Exec key of a .desktop file. Per the Desktop Entry spec,
/// inside the `"` quotes, `"`, `` ` ``, `$` and `\` get a leading backslash, and since the value is also
/// a string, those backslashes are written doubled in the file.
fn exec_quote(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Program that an `Exec=` line launches (the first argument, quoted or not).
fn exec_program(exec: &str) -> Option<PathBuf> {
    let value = exec.strip_prefix("Exec=")?;
    let Some(quoted) = value.strip_prefix('"') else {
        return value.split_whitespace().next().map(PathBuf::from);
    };
    // First the string level (\\ -> \), then the quoting level (\x -> x)
    let mut unescaped = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => unescaped.push(chars.next()?),
            _ => unescaped.push(c),
        }
    }
    let mut path = String::new();
    let mut chars = unescaped.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => path.push(chars.next()?),
            '"' => return Some(PathBuf::from(path)),
            _ => path.push(c),
        }
    }
    None
}

/// What to launch to open this copy: the AppImage if running from one (the executable
/// lives in a temporary mount), or the executable itself. $APPIMAGE only counts if this executable
/// is inside its mount ($APPDIR): the variable is inherited by everything another AppImage launches.
fn app_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    match (env_dir("APPIMAGE"), env_dir("APPDIR")) {
        (Some(image), Some(dir)) if exe.starts_with(&dir) => Some(image),
        _ => Some(exe),
    }
}

/// A Nogenuity AppImage (they carry the version in the name: Nogenuity-0.2.0-x86_64.AppImage).
fn is_nogenuity_appimage(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("AppImage"))
        && path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("nogenuity"))
}

fn startup_exec() -> Option<String> {
    Some(format!("Exec={} --tray", exec_quote(&app_exe()?)))
}

/// Program the session startup entry points to, if it exists.
fn autostart_target() -> Option<PathBuf> {
    let text = std::fs::read_to_string(autostart_file()?).ok()?;
    text.lines().find_map(exec_program)
}

/// If the app starts with the session but the entry points to another executable, points it to this
/// one: always with `force` (install.sh uses it after installing); otherwise, when that one no longer
/// exists or is another Nogenuity AppImage (on upgrading to a new version, the file name changes).
pub fn refresh_autostart(force: bool) {
    let (Some(target), Some(exe)) = (autostart_target(), app_exe()) else { return };
    let other_appimage = is_nogenuity_appimage(&exe) && is_nogenuity_appimage(&target);
    if target != exe && (force || !target.exists() || other_appimage) {
        set_autostart(true);
    }
}

pub fn autostart_enabled() -> bool {
    let (Some(file), Some(exec)) = (autostart_file(), startup_exec()) else { return false };
    std::fs::read_to_string(file).is_ok_and(|text| text.lines().any(|l| l == exec))
}

pub fn set_autostart(on: bool) {
    let Some(file) = autostart_file() else { return };
    if !on {
        let _ = std::fs::remove_file(file);
        return;
    }
    let Some(exec) = startup_exec() else { return };
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    let _ = std::fs::write(
        file,
        format!(
            "[Desktop Entry]\nType=Application\nName=Nogenuity\nComment=Open control for HyperX headsets\nIcon=nogenuity\n{exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_report_lengths() {
        // Vendor page; input 0x06 of 20 bytes, output 0x06 of 20 and feature 0xFF of 63 + ID
        let desc = [
            0x06, 0x00, 0xFF, 0x09, 0x01, 0xA1, 0x01, // Usage Page 0xFF00, Usage 1, Collection
            0x75, 0x08, 0x85, 0x06, 0x95, 0x13, 0x09, 0x01, 0x81, 0x02, // Report Size 8, ID 6, Count 19, Input
            0x95, 0x13, 0x09, 0x01, 0x91, 0x02, // Count 19, Output
            0x85, 0xFF, 0x96, 0x3F, 0x00, 0x09, 0x02, 0xB1, 0x02, // ID 0xFF, Count 63 (2 bytes), Feature
            0xC0,
        ];
        assert_eq!(feature_lengths(&desc), vec![(0xFF, 64)]);
        assert!(feature_lengths(&[0x06, 0x00, 0xFF, 0xC0]).is_empty());
        // Push / Pop: the size goes back to the saved one
        let pushed = [0x75, 0x08, 0x85, 0x01, 0xA4, 0x75, 0x10, 0x95, 0x01, 0xB1, 0x02, 0xB4, 0x95, 0x03, 0xB1, 0x02];
        assert_eq!(feature_lengths(&pushed), vec![(0x01, 6)]); // 16 + 24 bits + ID
    }

    #[test]
    fn exec_quoting_round_trips() {
        for p in ["/home/ana/.local/bin/nogenuity", "/home/Ana María/bin/nogenuity", "/tmp/a\"b$c`d\\e/nogenuity"] {
            let exec = format!("Exec={} --tray", exec_quote(Path::new(p)));
            assert_eq!(exec_program(&exec), Some(PathBuf::from(p)), "{exec}");
        }
        assert_eq!(exec_quote(Path::new("/a b")), "\"/a b\"");
        assert_eq!(exec_program("Exec=/usr/bin/nogenuity --tray"), Some(PathBuf::from("/usr/bin/nogenuity")));
    }

    #[test]
    fn nogenuity_appimages_are_recognised() {
        assert!(is_nogenuity_appimage(Path::new("/home/a/Apps/Nogenuity-0.2.0-x86_64.AppImage")));
        assert!(!is_nogenuity_appimage(Path::new("/home/a/Apps/VSCodium.AppImage")));
        assert!(!is_nogenuity_appimage(Path::new("/usr/bin/nogenuity")));
    }

    #[test]
    fn grant_script_is_fixed_text() {
        for file in [UDEV_RULE, AUDIO_PATH, AUDIO_PROFILE] {
            assert!(!file.contains('\''), "these files cannot contain single quotes");
        }
        let script = grant_script();
        assert!(script.contains("ATTRS{idVendor}==\"0951\""));
        assert!(script.contains("> /etc/udev/rules.d/70-nogenuity.rules &&"));
        assert!(!script.contains('$') && !script.contains('`'), "nothing the shell would expand");
        let syntax = std::process::Command::new("sh").args(["-n", "-c", &script]).status().unwrap();
        assert!(syntax.success(), "the root script has a syntax error");
    }

    #[test]
    fn grant_script_writes_the_audio_profile() {
        let root = std::env::temp_dir().join(format!("nogenuity-grant-{}", std::process::id()));
        std::fs::create_dir_all(root.join("etc/udev/rules.d")).unwrap();
        let script = grant_script()
            .replace("> /etc/", &format!("> {}/etc/", root.display()))
            .replace("mkdir -p /etc/alsa-card-profile/mixer/paths /etc/", &format!("mkdir -p {0}/etc/alsa-card-profile/mixer/paths {0}/etc/", root.display()))
            .replace("sed -i 's/, GROUP=\"plugdev\"//' /etc/", &format!("sed -i 's/, GROUP=\"plugdev\"//' {}/etc/", root.display()))
            .replace("udevadm control --reload-rules && udevadm trigger --subsystem-match=hidraw --action=change", "true");
        let status = std::process::Command::new("sh").args(["-c", &script]).status().unwrap();
        assert!(status.success());
        let read = |path: &str| std::fs::read_to_string(root.join(path)).unwrap();
        assert_eq!(read("etc/alsa-card-profile/mixer/paths/nogenuity-alpha-s-game.conf"), AUDIO_PATH);
        assert_eq!(read("etc/alsa-card-profile/mixer/profile-sets/nogenuity-alpha-s.conf"), AUDIO_PROFILE);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// What was tested on a real headset: the rule picks this profile, Game plays on device 0 with
    /// the control of that device, and Chat on device 1.
    #[test]
    fn audio_profile_is_the_tested_one() {
        assert!(UDEV_RULE.contains("ATTRS{idVendor}==\"0951\", ATTRS{idProduct}==\"16ed\", ENV{ACP_PROFILE_SET}=\"nogenuity-alpha-s.conf\""));
        assert!(AUDIO_PATH.contains("[Element PCM]\n"));
        assert!(!AUDIO_PATH.contains("PCM,1]"));
        let mapping = |name: &str| AUDIO_PROFILE.split(&format!("[Mapping {name}]")).nth(1).unwrap().split("\n[").next().unwrap().to_string();
        let game = mapping("stereo-game-output");
        assert!(game.contains("hw:%f,0,0") && game.contains("paths-output = nogenuity-alpha-s-game"));
        assert!(mapping("stereo-chat-output").contains("hw:%f,1,0"));
        assert!(mapping("mono-chat-input").contains("hw:%f,0,0"));
    }

    #[test]
    fn grant_script_failure_is_not_a_pkexec_code() {
        // A failure inside the script exits with 3, never with 126/127 (those are pkexec's)
        let script = grant_script().replace("udevadm control", "command-that-does-not-exist control");
        let script = script.replace(UDEV_RULE_PATH, "/nonexistent-dir/rule");
        let code = std::process::Command::new("sh").args(["-c", &script]).status().unwrap().code();
        assert_eq!(code, Some(3));
    }
}
