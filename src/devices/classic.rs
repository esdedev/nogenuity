//! "Classic DSP" family (Cloud Flight S, Cloud II Wireless): the same DSP as the Cloud Alpha S,
//! with some differences in the commands. Details in docs/PROTOCOL.md.
//!
//! EXPERIMENTAL: not tested on hardware. Command sequences and timings are kept exact, because on
//! the Alpha S three details only showed up with the headset at hand. Battery and charging are
//! requested with the second command set and arrive on the interrupt endpoint.

use std::time::{Duration, Instant};

use super::every::Every;
use super::{Band, Caps, Headset, Model, Status};
use crate::hid::{q8_23, q8_8, HidError, HidLink, Res};
use crate::i18n::t;

const CAPS: Caps = Caps {
    surround: true,
    sidetone: true,
    eq_bands: 10,
    // The EQ runs inside the 7.1 DSP, as on the Alpha S (same commands)
    eq_needs_surround: true,
    battery: true,
    ..Caps::NONE
};

pub const FLIGHT_S: Model = Model {
    name: "HyperX Cloud Flight S",
    vid: 0x0951,
    pid: 0x16EA,
    interface: 5,
    report_len: 62,
    usage_page: 0,
    collection: 0,
    feature_reports: false,
    caps: CAPS,
    tested: false,
    connect: |model, link| Classic::boxed(model, link, &FLIGHT_S_V),
};

pub const CLOUD_II_WIRELESS: Model = Model {
    name: "HyperX Cloud II Wireless",
    vid: 0x0951,
    pid: 0x1718,
    interface: 3,
    report_len: 62,
    usage_page: 0,
    collection: 0,
    feature_reports: false,
    caps: Caps { mic_mute_status: true, ..CAPS },
    tested: false,
    connect: |model, link| Classic::boxed(model, link, &CLOUD_II_WIRELESS_V),
};

/// The same headset with the HP-era ID.
pub const CLOUD_II_WIRELESS_HP: Model = Model { vid: 0x03F0, pid: 0x0C8A, ..CLOUD_II_WIRELESS };

/// What changes between models of the family.
struct Variant {
    /// Byte 6 of the EQ packets for channel 0 (the Alpha S uses 4).
    eq_channel: u8,
    /// Read the reply to each EQ packet.
    eq_reads_reply: bool,
    /// Wait before reading the state.
    status_wait: Duration,
    /// Do a GET_REPORT (and discard it) after requesting battery or charging.
    battery_reads_report: bool,
}

const FLIGHT_S_V: Variant =
    Variant { eq_channel: 10, eq_reads_reply: false, status_wait: Duration::from_millis(50), battery_reads_report: false };
const CLOUD_II_WIRELESS_V: Variant =
    Variant { eq_channel: 10, eq_reads_reply: true, status_wait: Duration::from_millis(200), battery_reads_report: true };

const REPORT_ID: u8 = 0x06;
const MOD_CTRL: [u8; 3] = [0x68, 0x4A, 0x8E];
const MOD_EQ: [u8; 3] = [0x30, 0x2B, 0xAE];
/// Second command set (byte 4 = 0x9A, byte 14 = 0xBB, byte 15 = subcommand).
const SUB_BATTERY: u8 = 0x02;
const SUB_CHARGING: u8 = 0x03;
const SUB_SIDETONE: u8 = 0x19;

/// Timeout for each reply from the interrupt endpoint.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1000);
/// Battery and charging change slowly: they are asked at most every 30 s.
const BATTERY_EVERY: Duration = Duration::from_secs(30);

/// Box buttons report (as on the Alpha S): its byte 2 holds flags.
const BUTTON_REPORT: u8 = 0x0A;

/// Control packet over an empty report: type in byte 2, byte 4 and module in bytes 7-9.
fn packet(mut p: Vec<u8>, kind: u8, b4: u8, module: [u8; 3]) -> Vec<u8> {
    p[2] = kind;
    p[4] = b4;
    p[7..10].copy_from_slice(&module);
    p
}

/// Second-set command: 06 00 02 00 9A 00 00 68 4A 8E 0A 00 00 00 BB <sub> <value>.
fn sub_packet(report: Vec<u8>, sub: u8, value: u8) -> Vec<u8> {
    let mut p = packet(report, 0x02, 0x9A, MOD_CTRL);
    p[10] = 0x0A;
    p[14] = 0xBB;
    p[15] = sub;
    p[16] = value;
    p
}

/// Second command set reply on the interrupt endpoint: byte 2 = 0xBB, byte 3 = subcommand. The
/// report ID is not otherwise checked; the buttons report is discarded, since its byte 2 could be
/// 0xBB by chance.
fn is_sub_reply(sub: u8, r: &[u8]) -> bool {
    r.first() != Some(&BUTTON_REPORT) && r.get(2) == Some(&0xBB) && r.get(3) == Some(&sub)
}

/// Battery % (byte 7), if it makes sense.
fn battery_of(r: &[u8]) -> Option<u8> {
    r.get(7).copied().filter(|&b| b <= 100)
}

/// Charging (byte 4): 1 = charging; 0 not, and 2 / 3 are two other states (full? error?).
fn charging_of(r: &[u8]) -> Option<bool> {
    r.get(4).map(|&v| v == 1)
}

struct Classic {
    model: &'static Model,
    link: HidLink,
    v: &'static Variant,
    /// The interrupt endpoint reader could be opened (without it no battery arrives).
    reader: bool,
    /// Last battery and charging (either may be missing).
    battery: Every<(Option<u8>, Option<bool>)>,
}

impl Classic {
    fn boxed(model: &'static Model, link: HidLink, v: &'static Variant) -> Box<dyn Headset> {
        // The battery arrives on the interrupt endpoint: start receiving right away
        let reader = link.start_reader();
        Box::new(Classic { model, link, v, reader, battery: Every::new(BATTERY_EVERY) })
    }

    fn report(&self) -> Vec<u8> {
        self.link.report(REPORT_ID)
    }

    /// Send, wait and collect one reply (without checking the echo, which is unknown).
    fn exchange(&self, p: &[u8], wait: Duration, read: bool) -> Res<Option<Vec<u8>>> {
        let _lock = self.link.lock()?;
        self.link.send(p, REPORT_ID)?;
        std::thread::sleep(wait);
        Ok(if read { self.link.read(REPORT_ID) } else { None })
    }

    fn sub_command(&self, sub: u8, value: u8) -> Res<()> {
        self.exchange(&sub_packet(self.report(), sub, value), Duration::from_millis(100), true)?;
        Ok(())
    }

    /// Requests battery and charging (50 ms after each request and, on the Cloud II Wireless, a
    /// discarded GET_REPORT) and collects both replies from the interrupt endpoint in a single
    /// wait. Err if it could not ask (something transient).
    fn ask_battery(&self) -> Res<(Option<u8>, Option<bool>)> {
        if !self.reader {
            return Ok((None, None));
        }
        let _lock = self.link.lock()?;
        self.link.drain_reader();
        for sub in [SUB_BATTERY, SUB_CHARGING] {
            self.link.send(&sub_packet(self.report(), sub, 0), REPORT_ID)?;
            std::thread::sleep(Duration::from_millis(50));
            if self.v.battery_reads_report {
                let _ = self.link.read(REPORT_ID);
            }
        }
        let (mut battery, mut charging) = (None, None);
        let deadline = Instant::now() + REPLY_TIMEOUT;
        while battery.is_none() || charging.is_none() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            let is_reply = |r: &[u8]| is_sub_reply(SUB_BATTERY, r) || is_sub_reply(SUB_CHARGING, r);
            let Some(r) = self.link.read_reply(left, is_reply) else { break };
            if is_sub_reply(SUB_BATTERY, &r) {
                battery = Some(battery_of(&r));
            } else {
                charging = Some(charging_of(&r));
            }
        }
        Ok((battery.flatten(), charging.flatten()))
    }

    /// Battery and charging, from the cache if recent.
    fn battery(&self) -> (Option<u8>, Option<bool>) {
        self.battery.get(|| self.ask_battery().ok()).unwrap_or_default()
    }

    /// Device state request: 06 00 00 00 FF 00 00 68 4A 8E. Without the battery: the switches read
    /// it before changing and must not wait for it.
    fn state(&self) -> Res<Status> {
        let p = packet(self.report(), 0x00, 0xFF, MOD_CTRL);
        let r = self.exchange(&p, self.v.status_wait, true)?.ok_or(HidError(t().no_status))?;
        if r.len() < 22 {
            return Err(HidError(t().short_status));
        }
        Ok(Status {
            surround: r[12] & 0x02 != 0,
            // Sidetone assumed on bit 4 (on the Alpha S it turned out to be bit 0)
            sidetone: r[14] & 0x10 != 0,
            mic_muted: self.model.caps.mic_mute_status && r[14] & 0x02 != 0,
            game_db: q8_8(&r, 16),
            chat_db: q8_8(&r, 20),
            ..Status::default()
        })
    }
}

impl Headset for Classic {
    fn model(&self) -> &'static Model {
        self.model
    }

    fn firmware(&self) -> u16 {
        self.link.firmware
    }

    fn status(&self) -> Res<Status> {
        let state = self.state()?;
        let (battery, charging) = self.battery();
        Ok(Status { battery, charging: charging.unwrap_or(false), ..state })
    }

    /// DSP mode: parameter 1 with the state in byte 14, then 2 and 8.
    fn set_surround(&self, on: bool) -> Res<bool> {
        if self.state()?.surround == on {
            return Ok(false);
        }
        let mut p = packet(self.report(), 0x0D, 0x9B, MOD_CTRL);
        p[5] = 1;
        p[14] = on as u8;
        for param in [0x01, 0x02, 0x08] {
            p[10] = param;
            self.exchange(&p, Duration::from_millis(50), true)?;
        }
        Ok(true)
    }

    /// Sets the sidetone (does not toggle, unlike the Alpha S).
    fn set_sidetone(&self, on: bool) -> Res<bool> {
        if self.state()?.sidetone == on {
            return Ok(false);
        }
        self.sub_command(SUB_SIDETONE, on as u8)?;
        Ok(true)
    }

    /// Like the Alpha S EQ packet, with another channel number and no known ACK.
    fn eq_band(&self, index: usize, band: Band) -> Res<bool> {
        let (hz, gain, q, kind) = band;
        for channel in 0..2u8 {
            let mut p = packet(self.report(), 0x0D, 0x41, MOD_EQ);
            p[6] = self.v.eq_channel + channel;
            p[10..14].copy_from_slice(&(index as i32).to_le_bytes());
            p[14..18].copy_from_slice(&(kind as i32).to_le_bytes());
            p[18..22].copy_from_slice(&(hz.round() as i32).to_le_bytes());
            p[22..26].copy_from_slice(&q8_23(q));
            p[26..30].copy_from_slice(&q8_23(gain));
            self.exchange(&p, Duration::from_millis(50), self.v.eq_reads_reply)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_replies() {
        // 0B .. BB 02 .. .. .. <battery> (the report ID is not checked)
        let r = [0x0B, 0, 0xBB, 0x02, 0, 0, 0, 73];
        assert!(is_sub_reply(SUB_BATTERY, &r) && !is_sub_reply(SUB_CHARGING, &r));
        assert_eq!(battery_of(&r), Some(73));
        assert_eq!(battery_of(&[0x0B, 0, 0xBB, 0x02, 0, 0, 0, 255]), None);
        assert_eq!(battery_of(&[0x0B, 0, 0xBB, 0x02]), None);
        // Charging: only 1 means charging
        assert_eq!(charging_of(&[0x0B, 0, 0xBB, 0x03, 1]), Some(true));
        assert_eq!(charging_of(&[0x0B, 0, 0xBB, 0x03, 2]), Some(false));
        // A buttons report (0A) is not the reply, even if its byte 2 is 0xBB
        assert!(!is_sub_reply(SUB_BATTERY, &[0x0A, 0, 0xBB, 0x02, 0, 0, 0, 50]));
        assert!(FLIGHT_S.caps.battery && CLOUD_II_WIRELESS_HP.caps.battery);
    }

    #[test]
    fn battery_request_bytes() {
        // Battery request, byte by byte
        let mut report = vec![0u8; 62];
        report[0] = REPORT_ID;
        let p = sub_packet(report, SUB_BATTERY, 0);
        assert_eq!(p[..17], [0x06, 0, 0x02, 0, 0x9A, 0, 0, 0x68, 0x4A, 0x8E, 0x0A, 0, 0, 0, 0xBB, 0x02, 0]);
        assert!(p[17..].iter().all(|&b| b == 0));
    }
}
