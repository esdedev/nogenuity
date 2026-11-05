//! HyperX Cloud Alpha S (USB 0951:16ED): protocol of its control box. Details in
//! docs/PROTOCOL.md.
//!
//! - Vendor HID collection (usage page 0xFF13) on interface 5, report 0x06 of 62 bytes.
//! - Byte 4 = 0x9B (control), bytes 7..9 = module id, byte 10 = parameter.
//! - 0x09 reads the state. 0x02 (7.1) and 0x07 (sidetone) TOGGLE like the physical button:
//!   byte 5 is ignored, so to set a state it must be read first.
//! - The EQ (module 30 2B AE) is only audible with 7.1 on.

use std::ops::Range;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{Band, ButtonEvent, Caps, Headset, Model, Status};
use crate::hid::{log, q8_23, q8_8, HidError, HidLink, Res};
use crate::i18n::t;

pub const MODEL: Model = Model {
    name: "HyperX Cloud Alpha S",
    vid: 0x0951,
    pid: 0x16ED,
    interface: 5,
    report_len: 62,
    usage_page: 0,
    collection: 0,
    feature_reports: false,
    caps: Caps {
        surround: true,
        sidetone: true,
        mic_mute_status: true,
        eq_bands: 10,
        eq_needs_surround: true,
        box_levels: true,
        game_chat_outputs: Some("Cloud Alpha S"),
        ..Caps::NONE
    },
    tested: true,
    connect: |model, link| Box::new(AlphaS { model, link }),
};

/// The same box with the HP-era ID. Same interface 5, but nobody has tested it yet with this
/// application.
pub const MODEL_HP: Model = Model { vid: 0x03F0, pid: 0x0C91, tested: false, ..MODEL };

const REPORT_ID: u8 = 0x06;
const BUTTON_REPORT: u8 = 0x0A;
const MOD_CTRL: [u8; 3] = [0x68, 0x4A, 0x8E];
const MOD_EQ: [u8; 3] = [0x30, 0x2B, 0xAE];
const PARAM_STATUS: u8 = 0x09;
const PARAM_SURROUND: u8 = 0x02;
const PARAM_SIDETONE: u8 = 0x07;
const DSP_SETTLE: Duration = Duration::from_millis(1500);

// The packet for one EQ band: see "Equalizer" in docs/PROTOCOL.md.
const EQ_KIND: u8 = 0x0D;
/// Byte 4, which the ACK echoes.
const EQ_COMMAND: u8 = 0x41;
/// Byte 6 is this plus the channel, and the ACK echoes it too.
const EQ_FIRST_CHANNEL: u8 = 4;
const EQ_CHANNELS: u8 = 2;
const EQ_BAND_INDEX: Range<usize> = 10..14;
const EQ_FILTER: Range<usize> = 14..18;
const EQ_FREQUENCY: Range<usize> = 18..22;
const EQ_Q: Range<usize> = 22..26;
const EQ_GAIN: Range<usize> = 26..30;

/// Writes one band's values into its packet.
fn write_eq_band(p: &mut [u8], index: usize, (hz, gain, q, filter): Band) {
    p[EQ_BAND_INDEX].copy_from_slice(&(index as i32).to_le_bytes());
    p[EQ_FILTER].copy_from_slice(&(filter as i32).to_le_bytes());
    p[EQ_FREQUENCY].copy_from_slice(&(hz.round() as i32).to_le_bytes());
    p[EQ_Q].copy_from_slice(&q8_23(q));
    p[EQ_GAIN].copy_from_slice(&q8_23(gain));
}

struct AlphaS {
    model: &'static Model,
    link: HidLink,
}

/// Instant from which the DSP accepts EQ after a 7.1 change (shared across threads).
fn dsp_ready_at() -> &'static Mutex<Option<Instant>> {
    static READY: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    READY.get_or_init(|| Mutex::new(None))
}

impl AlphaS {
    fn packet(&self, kind: u8, b4: u8, module: [u8; 3], b6: u8) -> Vec<u8> {
        let mut p = self.link.report(REPORT_ID);
        p[2] = kind;
        p[4] = b4;
        p[6] = b6;
        p[7..10].copy_from_slice(&module);
        p
    }

    /// Waits for the command ACK: same bytes 4 and 6, with bit 7 of byte 5 set.
    fn read_ack(&self, b4: u8, b6: u8) -> Option<Vec<u8>> {
        self.link.read_ack(REPORT_ID, |r| r[4] == b4 && r[6] == b6 && r[5] & 0x80 != 0)
    }

    fn control(&self, kind: u8, param: u8) -> Res<Option<Vec<u8>>> {
        let mut p = self.packet(kind, 0x9B, MOD_CTRL, 0);
        p[10] = param;
        let _lock = self.link.lock()?;
        self.link.send(&p, REPORT_ID)?;
        std::thread::sleep(Duration::from_millis(20));
        Ok(self.read_ack(0x9B, 0))
    }
}

impl Headset for AlphaS {
    fn model(&self) -> &'static Model {
        self.model
    }

    fn firmware(&self) -> u16 {
        self.link.firmware
    }

    fn status(&self) -> Res<Status> {
        let r = self.control(0x01, PARAM_STATUS)?.ok_or(HidError(t().no_status))?;
        if r.len() < 22 {
            return Err(HidError(t().short_status));
        }
        Ok(Status {
            surround: r[12] & 0x02 != 0,
            sidetone: r[14] & 0x01 != 0,
            mic_muted: r[14] & 0x02 != 0,
            game_db: q8_8(&r, 16),
            chat_db: q8_8(&r, 20),
            battery: None,
            charging: false,
            asleep: false,
            sidetone_level: None,
            auto_off: None,
            voice_prompts: None,
        })
    }

    fn set_surround(&self, on: bool) -> Res<bool> {
        if self.status()?.surround == on {
            return Ok(false);
        }
        self.control(0x0D, PARAM_SURROUND)?;
        *dsp_ready_at().lock().unwrap() = Some(Instant::now() + DSP_SETTLE);
        Ok(true)
    }

    fn set_sidetone(&self, on: bool) -> Res<bool> {
        if self.status()?.sidetone == on {
            return Ok(false);
        }
        self.control(0x01, PARAM_SIDETONE)?;
        Ok(true)
    }

    /// One band on both channels. True if the box ACKed both.
    fn eq_band(&self, index: usize, band: Band) -> Res<bool> {
        let ready = *dsp_ready_at().lock().unwrap();
        if let Some(t) = ready {
            let now = Instant::now();
            if t > now {
                log(&format!("   waiting {} ms for the DSP", (t - now).as_millis()));
                std::thread::sleep(t - now);
            }
        }
        let mut ok = true;
        for channel in 0..EQ_CHANNELS {
            let target = EQ_FIRST_CHANNEL + channel;
            let mut p = self.packet(EQ_KIND, EQ_COMMAND, MOD_EQ, target);
            write_eq_band(&mut p, index, band);
            let _lock = self.link.lock()?;
            self.link.send(&p, REPORT_ID)?;
            ok &= self.read_ack(EQ_COMMAND, target).is_some();
        }
        Ok(ok)
    }

    fn wait_button(&self) -> Option<ButtonEvent> {
        loop {
            let r = self.link.read_interrupt()?;
            if r[0] == BUTTON_REPORT && r.len() >= 12 {
                return Some(ButtonEvent { surround: r[2] & 0x02 != 0, game_db: q8_8(&r, 6), chat_db: q8_8(&r, 10) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the table in docs/PROTOCOL.md: 1 kHz low shelf at Q 1, -3 dB, as band 3.
    #[test]
    fn eq_band_bytes() {
        let mut p = vec![0u8; 62];
        write_eq_band(&mut p, 3, (1000.0, -3.0, 1.0, 1));
        assert_eq!(
            p[10..30],
            [
                0x03, 0x00, 0x00, 0x00, // band index
                0x01, 0x00, 0x00, 0x00, // filter
                0xE8, 0x03, 0x00, 0x00, // frequency
                0x00, 0x00, 0x80, 0x00, // Q 1.0, Q8.23
                0x00, 0x00, 0x80, 0xFE, // gain -3.0, Q8.23
            ]
        );
        assert!(p[30..].iter().all(|&b| b == 0));
    }
}
