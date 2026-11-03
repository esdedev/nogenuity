//! Wired Cloud III: mic monitoring and its level, with feature reports `20 <cmd> <value>`. Details
//! in docs/PROTOCOL.md.
//!
//! EXPERIMENTAL: cross-checked with HeadsetControl (which only sends `20 86`), not tested on
//! hardware yet.

use std::cell::Cell;
use std::time::Duration;

use super::every::Every;
use super::{Caps, Headset, Model, Status};
use crate::hid::{HidError, HidLink, Res};
use crate::i18n::t;

pub const CLOUD_III: Model = Model {
    name: "HyperX Cloud III",
    vid: 0x03F0,
    pid: 0x089D,
    interface: 3,
    // HeadsetControl: 62 bytes (the descriptor gives the size too)
    report_len: 62,
    // Below the 0xFF00 vendor range, so it has to be asked for by name
    usage_page: 0x01C0,
    collection: 2,
    feature_reports: true,
    caps: Caps { sidetone: true, sidetone_level_max: GAIN_MAX - GAIN_MIN, ..Caps::NONE },
    tested: false,
    connect: |model, link| {
        Box::new(Cloud3Wired { model, link, sidetone: Cell::new(None), level: Every::new(LEVEL_EVERY) })
    },
};
/// The revision HeadsetControl calls "Cloud 3 Wired 2026": its collection number is unknown, so the
/// usage page finds it.
pub const CLOUD_III_2: Model = Model { pid: 0x03CC, collection: 0, ..CLOUD_III };
/// Another revision. That it has the same interface and commands as the others is an assumption
/// until someone who owns one confirms it.
pub const CLOUD_III_3: Model = Model { pid: 0x0ACE, collection: 0, ..CLOUD_III };

const REPORT_ID: u8 = 0x20;
const GET_GAIN: u8 = 0x05;
const GET_SIDETONE: u8 = 0x06;
const SET_GAIN: u8 = 0x85;
const SET_SIDETONE: u8 = 0x86;
const SAVE_GAIN: u8 = 0x87;
/// Mic monitoring gains the headset accepts.
const GAIN_MIN: u8 = 0xC4;
const GAIN_MAX: u8 = 0xFA;
/// Gain set when mic monitoring is turned on and the headset has none in range.
const GAIN: u8 = 0xF0;
/// Always wait 500 ms before reading the reply to a query.
const READ_WAIT: Duration = Duration::from_millis(500);
/// The level only changes from the app, and reading it costs another 500 ms.
const LEVEL_EVERY: Duration = Duration::from_secs(60);

/// `20 <cmd> <value>` request, sized to the feature report.
fn request(mut p: Vec<u8>, command: u8, value: u8) -> Vec<u8> {
    p[1] = command;
    p[2] = value;
    p
}

/// Level (0..=Caps::sidetone_level_max) of a gain, if it is in range.
fn level_of(gain: u8) -> Option<u8> {
    (GAIN_MIN..=GAIN_MAX).contains(&gain).then(|| gain - GAIN_MIN)
}

/// Gain of a level.
fn gain_of(level: u8) -> u8 {
    GAIN_MIN + level.min(GAIN_MAX - GAIN_MIN)
}

struct Cloud3Wired {
    model: &'static Model,
    link: HidLink,
    /// Last mic monitoring state read or set: reading it costs 500 ms, so the switch does not
    /// read it again.
    sidetone: Cell<Option<bool>>,
    /// Mic monitoring level: None inside if the headset's gain is out of range.
    level: Every<Option<u8>>,
}

impl Cloud3Wired {
    fn send(&self, command: u8, value: u8) -> Res<()> {
        let _lock = self.link.lock()?;
        self.link.set_feature(&request(self.link.feature(REPORT_ID), command, value))
    }

    /// Query: SET_FEATURE with the command, wait and GET_FEATURE; the value is in byte 2.
    fn get(&self, command: u8) -> Res<u8> {
        let _lock = self.link.lock()?;
        self.link.set_feature(&request(self.link.feature(REPORT_ID), command, 0))?;
        std::thread::sleep(READ_WAIT);
        let r = self.link.get_feature(REPORT_ID).ok_or(HidError(t().no_status))?;
        r.get(2).copied().ok_or(HidError(t().short_status))
    }
}

impl Headset for Cloud3Wired {
    fn model(&self) -> &'static Model {
        self.model
    }

    fn firmware(&self) -> u16 {
        self.link.firmware
    }

    fn status(&self) -> Res<Status> {
        let on = self.get(GET_SIDETONE)? == 1;
        self.sidetone.set(Some(on));
        let level = self.level.get(|| self.get(GET_GAIN).ok().map(level_of)).flatten();
        Ok(Status { sidetone: on, sidetone_level: level, ..Status::default() })
    }

    /// When turning it on, if the gain is out of range, set and save `GAIN`; then the switch.
    fn set_sidetone(&self, on: bool) -> Res<bool> {
        let current = match self.sidetone.get() {
            Some(on) => on,
            None => self.status()?.sidetone,
        };
        if current == on {
            return Ok(false);
        }
        if on && level_of(self.get(GET_GAIN)?).is_none() {
            self.send(SET_GAIN, GAIN)?;
            self.send(SAVE_GAIN, 0x01)?;
            self.level.set(level_of(GAIN));
        }
        self.send(SET_SIDETONE, on as u8)?;
        self.sidetone.set(Some(on));
        Ok(true)
    }

    /// Not saved on the headset: a drag sends many of these, and the app sets it again on connect.
    fn set_sidetone_level(&self, level: u8) -> Res<()> {
        let gain = gain_of(level);
        self.send(SET_GAIN, gain)?;
        self.level.set(level_of(gain));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        let mut report = vec![0u8; 62];
        report[0] = REPORT_ID;
        // HeadsetControl: [0x20, 0x86, on]
        assert_eq!(request(report.clone(), SET_SIDETONE, 1)[..4], [0x20, 0x86, 0x01, 0x00]);
        assert_eq!(request(report, SET_GAIN, GAIN)[..3], [0x20, 0x85, 0xF0]);
    }

    #[test]
    fn level_maps_onto_the_gain_range() {
        let max = CLOUD_III.caps.sidetone_level_max;
        assert_eq!(gain_of(0), GAIN_MIN);
        assert_eq!(gain_of(max), GAIN_MAX);
        // Anything above the top stays on it
        assert_eq!(gain_of(u8::MAX), GAIN_MAX);
        for level in 0..=max {
            assert_eq!(level_of(gain_of(level)), Some(level));
        }
    }

    #[test]
    fn gains_out_of_range_have_no_level() {
        assert_eq!(level_of(GAIN_MIN - 1), None);
        assert_eq!(level_of(GAIN_MAX + 1), None);
        assert_eq!(level_of(0), None);
        assert!(level_of(GAIN).is_some());
    }

    #[test]
    fn revisions_share_everything_but_the_id() {
        for m in [&CLOUD_III_2, &CLOUD_III_3] {
            assert_eq!(m.usage_page, CLOUD_III.usage_page);
            assert_eq!(m.caps, CLOUD_III.caps);
            // Their collection number is unknown: the usage page finds it
            assert_eq!(m.collection, 0);
        }
    }
}
