//! "Basic" family: only battery, charging and whether the headset is on (the USB receiver stays
//! plugged in even when it is not). Cloud Flight Wireless (output report, reply on the interrupt
//! endpoint) and Cloud Stinger Core Wireless (feature reports). Details in docs/PROTOCOL.md.
//!
//! EXPERIMENTAL: not tested with this app on hardware. A tester checked the Stinger protocol on
//! Linux with their own tool (issue #6).

use std::cell::Cell;
use std::time::Duration;

use super::{Caps, Headset, Model, Status};
use crate::hid::{HidError, HidLink, Res};
use crate::i18n::t;

const CAPS: Caps = Caps { battery: true, ..Caps::NONE };

/// Approximate curves (V) of a 3.7 V lithium cell, discharging and charging: 0 %, 10 %, ..., 100 %.
const LI_DISCHARGING: [f32; 11] = [3.50, 3.65, 3.70, 3.74, 3.77, 3.79, 3.83, 3.90, 3.95, 4.04, 4.12];
const LI_CHARGING: [f32; 11] = [3.40, 3.83, 3.87, 3.92, 3.93, 3.95, 3.97, 4.00, 4.05, 4.09, 4.13];

/// Battery % from the voltage, interpolating between curve points (truncating). Below the first
/// point, 0; from the last one, 100.
fn lithium_percent(volts: f32, curve: &[f32; 11]) -> u8 {
    if volts >= curve[10] {
        return 100;
    }
    (0..10)
        .find(|&i| curve[i] <= volts && volts <= curve[i + 1])
        .map_or(0, |i| (i * 10) as u8 + ((volts - curve[i]) / (curve[i + 1] - curve[i]) * 10.0) as u8)
}

// --- Cloud Flight Wireless ---------------------------------------------------------------------

pub const CLOUD_FLIGHT_WIRELESS: Model = Model {
    name: "HyperX Cloud Flight Wireless",
    vid: 0x0951,
    pid: 0x16C4,
    interface: 3,
    // HeadsetControl and a USB capture: 20 bytes
    report_len: 20,
    usage_page: 0,
    // mi_03&col03; the same interface has another vendor collection (power and mic events)
    collection: 3,
    feature_reports: false,
    caps: CAPS,
    tested: false,
    connect: |model, link| Flight::boxed(model, link),
};
pub const CLOUD_FLIGHT_WIRELESS_2: Model = Model { pid: 0x1723, ..CLOUD_FLIGHT_WIRELESS };
pub const CLOUD_FLIGHT_WIRELESS_HP: Model = Model { vid: 0x03F0, pid: 0x0E90, ..CLOUD_FLIGHT_WIRELESS };

/// Battery request: output report 21 FF 05; the reply starts the same.
const FLIGHT_BATTERY: [u8; 3] = [0x21, 0xFF, 0x05];
/// No reply within 2 s means the headset is off.
const FLIGHT_TIMEOUT: Duration = Duration::from_millis(2000);

/// Battery % from the two reply bytes (voltage in mV, big-endian). Charging if the voltage is
/// between 0x1014 and 0x11FF (then it reads 100).
fn flight_battery(hi: u8, lo: u8) -> (u8, bool) {
    let mv = u16::from_be_bytes([hi, lo]);
    let charging = (0x1014..=0x11FF).contains(&mv);
    (if charging { 100 } else { lithium_percent(mv as f32 / 1000.0, &LI_DISCHARGING) }, charging)
}

struct Flight {
    model: &'static Model,
    link: HidLink,
    /// The interrupt endpoint reader could be opened (without it no reply arrives).
    reader: bool,
}

impl Flight {
    fn boxed(model: &'static Model, link: HidLink) -> Box<dyn Headset> {
        let reader = link.start_reader();
        Box::new(Flight { model, link, reader })
    }
}

impl Headset for Flight {
    fn model(&self) -> &'static Model {
        self.model
    }

    fn firmware(&self) -> u16 {
        self.link.firmware
    }

    /// Requests the battery; no reply means the headset is off (the receiver is present).
    fn status(&self) -> Res<Status> {
        if !self.reader {
            return Err(HidError(t().no_status));
        }
        let mut p = self.link.report(FLIGHT_BATTERY[0]);
        p[..3].copy_from_slice(&FLIGHT_BATTERY);
        let reply = {
            let _lock = self.link.lock()?;
            self.link.drain_reader();
            self.link.send(&p, FLIGHT_BATTERY[0])?;
            self.link.read_reply(FLIGHT_TIMEOUT, |r| r.starts_with(&FLIGHT_BATTERY) && r.len() >= 5)
        };
        Ok(match reply {
            Some(r) => {
                let (battery, charging) = flight_battery(r[3], r[4]);
                Status { battery: Some(battery), charging, ..Status::default() }
            }
            None => Status { asleep: true, ..Status::default() },
        })
    }
}

// --- Cloud Stinger Core Wireless -----------------------------------------------------------------

pub const CLOUD_STINGER_CORE_WIRELESS: Model = Model {
    name: "HyperX Cloud Stinger Core Wireless",
    vid: 0x0951,
    pid: 0x170B,
    interface: 3,
    // Trimmed to the descriptor's feature report size (on Linux it is read from it)
    report_len: 64,
    usage_page: 0,
    collection: 3,
    feature_reports: true,
    caps: CAPS,
    tested: false,
    connect: |model, link| Box::new(Stinger { model, link, shown: Cell::new(ChargeFilter::default()) }),
};

/// Feature report for all requests and replies.
const STINGER_ID: u8 = 0xFF;
/// Headset on? Reply: byte 11 = 1.
const STINGER_CONNECTIVITY: [u8; 20] =
    [0xFF, 0x07, 0x00, 0xFD, 0x04, 0x00, 0x00, 0x02, 0x82, 0x03, 0x05, 0x29, 0x0F, 0x00, 0x00, 0x03, 0x68, 0x10, 0x00, 0x00];
/// Battery (byte 6 = 0xF1 is the receiver variant, the only ID there is). Reply: byte 9 =
/// charge state, bytes 11-12 = voltage in mV (little-endian).
const STINGER_BATTERY: [u8; 12] = [0xFF, 0x07, 0x00, 0xFD, 0x04, 0x0C, 0xF1, 0x02, 0x01, 0x04, 0xF0, 0x0C];
/// How often GET_FEATURE is retried after SET_FEATURE, and for how long. The battery reply took
/// about 28 ms on a tester's unit, the connectivity one about 3 ms.
const STINGER_POLL: Duration = Duration::from_millis(5);
const STINGER_TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Charge {
    No,
    Charging,
    Full,
}

/// Charge state from byte 9 (3 / 5 / 6); any other value is ignored.
fn stinger_charge(b: u8) -> Option<Charge> {
    match b {
        0x03 => Some(Charge::No),
        0x05 => Some(Charge::Charging),
        0x06 => Some(Charge::Full),
        _ => None,
    }
}

/// Whether `reply` answers `request` and hasn't been read yet: byte 3 = 0xFE, byte 2 != 0 and bytes
/// 4-5 = request bytes 5 and 4. The dongle keeps returning a reply it already gave, with bytes 1-2
/// = 01 00, until the next one is ready.
fn stinger_is_fresh(reply: &[u8], request: &[u8]) -> bool {
    reply.len() > 5 && reply[3] == 0xFE && reply[2] != 0 && reply[4] == request[5] && reply[5] == request[4]
}

/// Charge state and voltage (mV) of a battery reply. None if it's short or the state is unknown.
fn stinger_reading(reply: &[u8]) -> Option<(Charge, i16)> {
    let (state, lo, hi) = (*reply.get(9)?, *reply.get(11)?, *reply.get(12)?);
    Some((stinger_charge(state)?, i16::from_le_bytes([lo, hi])))
}

/// Battery % from the voltage; when full, 100.
fn stinger_battery(volts: f32, charge: Charge) -> u8 {
    match charge {
        Charge::Full => 100,
        Charge::Charging => lithium_percent(volts, &LI_CHARGING),
        Charge::No => lithium_percent(volts, &LI_DISCHARGING),
    }
}

/// What is shown of the battery, filtered: when charging stops the reading may bounce for a few
/// seconds, so that change is only accepted after four equal readings in a row (starting to
/// charge, immediately). Meanwhile the previous value is kept, battery included.
#[derive(Clone, Copy, Debug, Default)]
struct ChargeFilter {
    shown: Option<(Charge, u8)>,
    pending: Option<Charge>,
    count: u8,
}

impl ChargeFilter {
    fn update(&mut self, charge: Charge, battery: u8) -> (Charge, u8) {
        let take = match self.shown {
            Some((shown, _)) if shown != charge && shown != Charge::No => {
                if self.pending == Some(charge) {
                    self.count += 1;
                } else {
                    (self.pending, self.count) = (Some(charge), 0);
                }
                self.count > 2
            }
            _ => true,
        };
        if take {
            *self = ChargeFilter { shown: Some((charge, battery)), ..ChargeFilter::default() };
        }
        self.shown.unwrap_or((charge, battery))
    }
}

struct Stinger {
    model: &'static Model,
    link: HidLink,
    shown: Cell<ChargeFilter>,
}

impl Stinger {
    /// SET_FEATURE with the request, then GET_FEATURE 0xFF until the fresh reply arrives.
    fn ask(&self, request: &[u8]) -> Res<Option<Vec<u8>>> {
        let mut p = self.link.feature(STINGER_ID);
        let n = request.len().min(p.len());
        p[..n].copy_from_slice(&request[..n]);
        let _lock = self.link.lock()?;
        self.link.set_feature(&p)?;
        Ok(self.link.poll_feature(STINGER_ID, STINGER_POLL, STINGER_TIMEOUT, |r| stinger_is_fresh(r, request)))
    }
}

impl Headset for Stinger {
    fn model(&self) -> &'static Model {
        self.model
    }

    fn firmware(&self) -> u16 {
        self.link.firmware
    }

    /// If the headset is on, the battery.
    fn status(&self) -> Res<Status> {
        let on = self.ask(&STINGER_CONNECTIVITY)?.is_some_and(|r| r.get(11) == Some(&1));
        if !on {
            // When it turns back on, the first reading is taken as is
            self.shown.set(ChargeFilter::default());
            return Ok(Status { asleep: true, ..Status::default() });
        }
        let reading = self.ask(&STINGER_BATTERY)?.as_deref().and_then(stinger_reading);
        let mut filter = self.shown.get();
        // No reply or an unknown charge state changes nothing
        let shown = match reading {
            Some((charge, mv)) => Some(filter.update(charge, stinger_battery(mv as f32 / 1000.0, charge))),
            None => filter.shown,
        };
        self.shown.set(filter);
        Ok(Status {
            battery: shown.map(|(_, b)| b),
            charging: shown.is_some_and(|(c, _)| c == Charge::Charging),
            ..Status::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lithium_curves() {
        for curve in [LI_DISCHARGING, LI_CHARGING] {
            assert!(curve.windows(2).all(|w| w[0] < w[1]));
            assert_eq!(lithium_percent(curve[0] - 0.1, &curve), 0);
            assert_eq!(lithium_percent(curve[10], &curve), 100);
        }
        // 3.80 V: a quarter of the way between 3.79 (50 %) and 3.83 (60 %)
        assert_eq!(lithium_percent(3.80, &LI_DISCHARGING), 52);
    }

    #[test]
    fn flight_battery_values() {
        assert_eq!(flight_battery(0x0D, 0x80), (0, false)); // 3456 mV
        assert_eq!(flight_battery(0x0F, 0x00), (61, false)); // 3840 mV
        assert_eq!(flight_battery(0x10, 0x13), (99, false)); // 4115 mV
        // Charging: from 0x1014 to 0x11FF
        assert_eq!(flight_battery(0x10, 0x14), (100, true));
        assert_eq!(flight_battery(0x11, 0xFF), (100, true));
        assert_eq!(flight_battery(0x12, 0x00), (100, false));
    }

    #[test]
    fn stinger_battery_values() {
        assert_eq!(stinger_battery(3.40, Charge::No), 0);
        assert_eq!(stinger_battery(3.80, Charge::No), 52);
        assert_eq!(stinger_battery(4.20, Charge::No), 100);
        assert_eq!(stinger_battery(4.0, Charge::Charging), 70);
        assert_eq!(stinger_battery(3.5, Charge::Full), 100);
        assert_eq!(stinger_charge(0x05), Some(Charge::Charging));
        assert_eq!(stinger_charge(0x04), None);
    }

    #[test]
    fn leaving_charge_is_debounced() {
        let mut f = ChargeFilter::default();
        assert_eq!(f.update(Charge::No, 40), (Charge::No, 40));
        // Starting to charge: immediately
        assert_eq!(f.update(Charge::Charging, 45), (Charge::Charging, 45));
        // Stopping charging: three readings keep the previous value, the fourth goes through
        for _ in 0..3 {
            assert_eq!(f.update(Charge::No, 41), (Charge::Charging, 45));
        }
        assert_eq!(f.update(Charge::No, 41), (Charge::No, 41));
        // A different reading in between restarts the count (full to charging is filtered too)
        f.update(Charge::Full, 100);
        f.update(Charge::No, 90);
        f.update(Charge::No, 90);
        f.update(Charge::Charging, 99);
        f.update(Charge::No, 90);
        assert_eq!(f.update(Charge::No, 90), (Charge::Full, 100));
    }

    // Replies captured on a Cloud Stinger Core Wireless (issue #6)
    const FRESH_CONNECTIVITY: [u8; 13] =
        [0xFF, 0x0C, 0x05, 0xFE, 0x00, 0x04, 0x0F, 0x05, 0x82, 0x00, 0xFB, 0x01, 0x0E];
    const READ_CONNECTIVITY: [u8; 13] =
        [0xFF, 0x01, 0x00, 0xFE, 0x00, 0x04, 0x0F, 0x05, 0x82, 0x00, 0xFB, 0x01, 0x0E];
    const FRESH_BATTERY: [u8; 20] = [
        0xFF, 0x12, 0x05, 0xFE, 0x0C, 0x04, 0x1F, 0x0B, 0x01, 0x03, 0x05, 0xCB, 0x0E, 0x00, 0x00, 0x03, 0x68, 0x10,
        0x00, 0x00,
    ];
    const FRESH_BATTERY_CHARGING: [u8; 20] = [
        0xFF, 0x12, 0x05, 0xFE, 0x0C, 0x04, 0x1F, 0x0B, 0x01, 0x05, 0x03, 0x7E, 0x0F, 0xE0, 0x02, 0x03, 0x68, 0x10,
        0x00, 0x00,
    ];
    const FRESH_BATTERY_FULL: [u8; 15] =
        [0xFF, 0x12, 0x05, 0xFE, 0x0C, 0x04, 0x1F, 0x0B, 0x01, 0x06, 0x05, 0xCF, 0x0F, 0x00, 0x00];
    const READ_BATTERY: [u8; 13] =
        [0xFF, 0x01, 0x00, 0xFE, 0x0C, 0x04, 0x1F, 0x0B, 0x01, 0x03, 0x05, 0xCC, 0x0E];

    #[test]
    fn a_fresh_reply_to_the_request_is_accepted() {
        assert!(stinger_is_fresh(&FRESH_CONNECTIVITY, &STINGER_CONNECTIVITY));
        assert!(stinger_is_fresh(&FRESH_BATTERY, &STINGER_BATTERY));
        assert!(stinger_is_fresh(&FRESH_BATTERY_CHARGING, &STINGER_BATTERY));
        assert!(stinger_is_fresh(&FRESH_BATTERY_FULL, &STINGER_BATTERY));
    }

    #[test]
    fn a_reply_already_read_is_rejected() {
        assert!(!stinger_is_fresh(&READ_CONNECTIVITY, &STINGER_CONNECTIVITY));
        assert!(!stinger_is_fresh(&READ_BATTERY, &STINGER_BATTERY));
        // What a fixed 20 ms wait got: the connectivity reply again after asking for the battery
        assert!(!stinger_is_fresh(&READ_CONNECTIVITY, &STINGER_BATTERY));
    }

    #[test]
    fn a_fresh_reply_to_another_request_is_rejected() {
        // Its byte 11 is part of the voltage, which would look like "headset off"
        assert!(!stinger_is_fresh(&FRESH_BATTERY, &STINGER_CONNECTIVITY));
        assert!(!stinger_is_fresh(&FRESH_CONNECTIVITY, &STINGER_BATTERY));
    }

    #[test]
    fn a_short_or_empty_reply_is_rejected() {
        assert!(!stinger_is_fresh(&[], &STINGER_BATTERY));
        assert!(!stinger_is_fresh(&FRESH_BATTERY[..5], &STINGER_BATTERY));
    }

    #[test]
    fn battery_replies_are_parsed_as_captured() {
        let (charge, mv) = stinger_reading(&FRESH_BATTERY).unwrap();
        assert_eq!((charge, mv), (Charge::No, 3787));
        let (charge, mv) = stinger_reading(&FRESH_BATTERY_CHARGING).unwrap();
        assert_eq!((charge, mv), (Charge::Charging, 3966));
        let (charge, mv) = stinger_reading(&FRESH_BATTERY_FULL).unwrap();
        assert_eq!((charge, mv), (Charge::Full, 4047));
        assert_eq!(stinger_battery(mv as f32 / 1000.0, charge), 100);
        // The stale reply a fixed wait got has an unknown state in byte 9
        assert_eq!(stinger_reading(&READ_CONNECTIVITY), None);
        assert_eq!(stinger_reading(&FRESH_BATTERY[..12]), None);
    }
}
