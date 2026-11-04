//! Generic HID link to a headset, shared by all models and by Windows and Linux.
//! Device access comes from each OS's `sys::Transport`; each model's protocol
//! lives in `devices/`.
//!
//! Learned with the Cloud Alpha S, valid for any control box that replies via GET_REPORT:
//! - Each command is "send + reply". The box won't accept a new command while it has an
//!   uncollected reply, and rejects sends (error 31): reading it with GET_REPORT unblocks it.
//!   So each command's ACK is awaited and, if a send fails, the reply is collected and it retries.
//! - Two processes interleaving packets confuse the firmware: a cross-process lock serializes them.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::i18n::t;
use crate::sys::{self, CommandLock, Transport};

#[derive(Debug)]
pub struct HidError(pub &'static str);

pub type Res<T> = Result<T, HidError>;

/// Log of every packet in hid.log, in the data folder (cleared when it exceeds 512 KB).
/// If the box stops responding, this shows what was sent to it right before.
pub fn log(line: &str) {
    use std::io::Write;
    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let file = FILE.get_or_init(|| {
        let dir = sys::data_dir()?;
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hid.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    });
    if let Some(f) = file {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let (secs, ms) = (now.as_secs() % 86_400, now.subsec_millis());
        let _ = writeln!(
            f.lock().unwrap(),
            "{:02}:{:02}:{:02}.{ms:03} [{:?}] {line}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60,
            std::thread::current().id()
        );
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

/// Signed Q8.8, big-endian.
pub fn q8_8(r: &[u8], offset: usize) -> f32 {
    i16::from_be_bytes([r[offset], r[offset + 1]]) as f32 / 256.0
}

/// Signed Q8.23, little-endian.
pub fn q8_23(v: f32) -> [u8; 4] {
    ((v as f64 * (1u32 << 23) as f64) as i32).to_le_bytes()
}

pub struct HidLink {
    transport: Transport,
    in_len: usize,
    out_len: usize,
    feature_len: usize,
    /// Size of each feature report by ID, if the OS provides it that way (Linux, from the descriptor).
    feature_lens: Vec<(u8, usize)>,
    pub firmware: u16,
}

impl HidLink {
    /// Opens control interface `interface` of `vid:pid`. `report_len` is used when the OS
    /// doesn't report the report sizes (Linux).
    pub fn open(
        vid: u16,
        pid: u16,
        interface: u8,
        usage_page: u16,
        collection: u8,
        feature: bool,
        report_len: usize,
    ) -> Option<HidLink> {
        let o = Transport::open(vid, pid, interface, usage_page, collection, feature)?;
        let len = |n: usize| if n > 0 { n } else { report_len };
        log(&format!("opened {vid:04x}:{pid:04x}, firmware {:04X}", o.firmware));
        Some(HidLink {
            transport: o.transport,
            in_len: len(o.in_len),
            out_len: len(o.out_len),
            feature_len: len(o.feature_len),
            feature_lens: o.feature_lens,
            firmware: o.firmware,
        })
    }

    /// Empty output report of the device's size, with its ID in byte 0.
    pub fn report(&self, id: u8) -> Vec<u8> {
        let mut p = vec![0u8; self.out_len];
        p[0] = id;
        p
    }

    /// Empty feature report of the device's size, with its ID in byte 0.
    pub fn feature(&self, id: u8) -> Vec<u8> {
        let len = self.feature_lens.iter().find(|(r, _)| *r == id).map_or(self.feature_len, |(_, n)| *n);
        let mut p = vec![0u8; len];
        p[0] = id;
        p
    }

    /// SET_FEATURE (report ID in byte 0).
    pub fn set_feature(&self, p: &[u8]) -> Res<()> {
        log(&format!("=> {}", hex(&p[..30.min(p.len())])));
        self.transport.set_feature(p).then_some(()).ok_or(HidError(t().rejected))
    }

    /// GET_FEATURE of report `id`.
    pub fn get_feature(&self, id: u8) -> Option<Vec<u8>> {
        let mut buf = self.feature(id);
        let ok = self.transport.get_feature(&mut buf);
        log(&if ok { format!("<= {}", hex(&buf[..24.min(buf.len())])) } else { "<= no reply".into() });
        ok.then_some(buf)
    }

    /// GET_FEATURE of report `id` every `every` until `is_reply` accepts one, or until `timeout`.
    /// Only the outcome goes to the log, so the retries don't push older lines out of the report.
    pub fn poll_feature(
        &self,
        id: u8,
        every: Duration,
        timeout: Duration,
        is_reply: impl Fn(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        let start = Instant::now();
        let mut last = None;
        let reply = poll(every, timeout, || {
            let mut buf = self.feature(id);
            if !self.transport.get_feature(&mut buf) {
                return None;
            }
            if is_reply(&buf) {
                return Some(buf);
            }
            last = Some(buf);
            None
        });
        let ms = start.elapsed().as_millis();
        log(&match (&reply, &last) {
            (Some(r), _) => format!("<= {} (after {ms} ms)", hex(&r[..24.min(r.len())])),
            (None, Some(r)) => format!("<= no fresh reply after {ms} ms, last {}", hex(&r[..24.min(r.len())])),
            (None, None) => format!("<= no reply after {ms} ms"),
        });
        reply
    }

    /// Serializes commands with other processes using the headset.
    pub fn lock(&self) -> Res<CommandLock> {
        CommandLock::take().ok_or(HidError(t().busy))
    }

    /// Sends a report; if the box rejects it, collects the pending reply and retries.
    pub fn send(&self, p: &[u8], reply_id: u8) -> Res<()> {
        log(&format!("-> {}", hex(&p[..30.min(p.len())])));
        if self.transport.write(p) {
            return Ok(());
        }
        for _ in 0..3 {
            log("   send rejected: collecting the pending reply");
            let _ = self.read(reply_id);
            if self.transport.write(p) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        log("   SEND REJECTED");
        Err(HidError(t().rejected))
    }

    /// GET_REPORT of input report `id`.
    pub fn read(&self, id: u8) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; self.in_len];
        buf[0] = id;
        let ok = self.transport.get_input(&mut buf);
        log(&if ok { format!("<- {}", hex(&buf[..24.min(buf.len())])) } else { "<- no reply".into() });
        ok.then_some(buf)
    }

    /// Reads until the reply matching `is_ack` arrives (about 200 ms at most).
    pub fn read_ack(&self, id: u8, is_ack: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
        for _ in 0..20 {
            if let Some(r) = self.read(id) {
                if is_ack(&r) {
                    return Some(r);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// For models that reply on the interrupt endpoint: starts receiving their reports
    /// (call before the first command).
    pub fn start_reader(&self) -> bool {
        let ok = self.transport.open_reader();
        if !ok {
            log("   could not open the interrupt endpoint reader");
        }
        ok
    }

    /// Discards reports already received (headset notifications between queries), so a
    /// stale reply isn't mistaken for the next request's.
    pub fn drain_reader(&self) {
        let mut buf = vec![0u8; self.in_len.max(64)];
        for _ in 0..64 {
            if self.transport.read_timeout(&mut buf, Duration::ZERO).is_none() {
                break;
            }
        }
    }

    /// Reads reports from the interrupt endpoint until one matches `is_reply`, or until `timeout`.
    /// Those that aren't the reply (headset notifications) are discarded.
    pub fn read_reply(&self, timeout: Duration, is_reply: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let mut buf = vec![0u8; self.in_len.max(64)];
            let n = self.transport.read_timeout(&mut buf, left)?;
            buf.truncate(n);
            log(&format!("<< {}", hex(&buf[..12.min(n)])));
            if is_reply(&buf) {
                return Some(buf);
            }
        }
    }

    /// Waits for the next report from the interrupt endpoint. None if disconnected.
    pub fn read_interrupt(&self) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; self.in_len];
        if !self.transport.read_report(&mut buf) {
            return None;
        }
        log(&format!("<< {}", hex(&buf[..12.min(buf.len())])));
        Some(buf)
    }
}

/// Waits `every` and calls `read`, again and again, until it gives a value or `timeout` has passed.
fn poll<T>(every: Duration, timeout: Duration, mut read: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        std::thread::sleep(every);
        if let Some(value) = read() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: Duration = Duration::from_millis(1);

    #[test]
    fn poll_keeps_reading_until_a_value_arrives() {
        let mut reads = 0;
        let value = poll(EVERY, Duration::from_secs(5), || {
            reads += 1;
            (reads == 3).then_some("fresh")
        });
        assert_eq!(value, Some("fresh"));
        assert_eq!(reads, 3);
    }

    #[test]
    fn poll_gives_up_after_the_timeout() {
        let timeout = Duration::from_millis(30);
        let start = Instant::now();
        let value: Option<u8> = poll(EVERY, timeout, || None);
        assert_eq!(value, None);
        assert!(start.elapsed() >= timeout);
    }

    #[test]
    fn poll_waits_before_the_first_read() {
        let every = Duration::from_millis(20);
        let start = Instant::now();
        assert_eq!(poll(every, Duration::from_secs(5), || Some(start.elapsed())).map(|t| t >= every), Some(true));
    }
}
