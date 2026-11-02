//! Wired Cloud III: only mic monitoring, with feature reports `20 <cmd> <value>`. Details in
//! docs/PROTOCOL.md.
//!
//! EXPERIMENTAL: cross-checked with HeadsetControl (which only sends `20 86`), not tested on
//! hardware yet.

use std::cell::Cell;
use std::time::Duration;

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
    usage_page: 0,
    collection: 2,
    feature_reports: true,
    caps: Caps { sidetone: true, ..Caps::NONE },
    tested: false,
    connect: |model, link| Box::new(Cloud3Wired { model, link, sidetone: Cell::new(None) }),
};
/// The revision HeadsetControl calls "Cloud 3 Wired 2026": its collection is unknown, so the first
/// one that matches is accepted.
pub const CLOUD_III_2: Model = Model { pid: 0x03CC, collection: 0, ..CLOUD_III };

const REPORT_ID: u8 = 0x20;
const GET_GAIN: u8 = 0x05;
const GET_SIDETONE: u8 = 0x06;
const SET_GAIN: u8 = 0x85;
const SET_SIDETONE: u8 = 0x86;
const SAVE_GAIN: u8 = 0x87;
/// Gain set when mic monitoring is turned on.
const GAIN: u8 = 0xF0;
/// Always wait 500 ms before reading the reply to a query.
const READ_WAIT: Duration = Duration::from_millis(500);

/// `20 <cmd> <value>` request, sized to the feature report.
fn request(mut p: Vec<u8>, command: u8, value: u8) -> Vec<u8> {
    p[1] = command;
    p[2] = value;
    p
}

struct Cloud3Wired {
    model: &'static Model,
    link: HidLink,
    /// Last mic monitoring state read or set: reading it costs 500 ms, so the switch does not
    /// read it again.
    sidetone: Cell<Option<bool>>,
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
        Ok(Status { sidetone: on, ..Status::default() })
    }

    /// When turning it on, if the gain is not `GAIN`, set and save it; then the switch.
    fn set_sidetone(&self, on: bool) -> Res<bool> {
        let current = match self.sidetone.get() {
            Some(on) => on,
            None => self.status()?.sidetone,
        };
        if current == on {
            return Ok(false);
        }
        if on && self.get(GET_GAIN)? != GAIN {
            self.send(SET_GAIN, GAIN)?;
            self.send(SAVE_GAIN, 0x01)?;
        }
        self.send(SET_SIDETONE, on as u8)?;
        self.sidetone.set(Some(on));
        Ok(true)
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
}
