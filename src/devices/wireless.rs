//! "Wireless control set" family: Cloud Alpha Wireless, Cloud II Wireless DTS (and Cloud Stinger 2
//! Wireless, same protocol) and Cloud III Wireless / Cloud II Core Wireless. All use request /
//! reply commands of the same shape, `<header> <command> <value>`, with different headers and
//! numbers, and reply on the interrupt endpoint (not via GET_REPORT). Details in
//! docs/PROTOCOL.md.
//!
//! EXPERIMENTAL: cross-checked with HeadsetControl, not tested on hardware yet.
//! Battery, charging, mic mute, mic monitoring, headset off and the headset card: auto-off on
//! all; mic monitoring volume on the Alpha Wireless and the `66` set; voice prompts only on the
//! Alpha Wireless (on the DTS the volume command is unused and HeadsetControl does not have it).

use std::time::Duration;

use super::every::Every;
use super::{Caps, Headset, Model, Status};
use crate::hid::{HidError, HidLink, Res};
use crate::i18n::t;

/// Auto-off minutes offered (0 = never). The Alpha Wireless and the DTS accept any number of
/// minutes (HeadsetControl: 0-255, 0 disables it); these are the usual ones.
const AUTO_OFF: &[u8] = &[0, 10, 20, 30, 60];
/// The `66` set: Never, 10, 20, 30.
const AUTO_OFF_66: &[u8] = &[0, 10, 20, 30];

/// What the model has, according to its protocol commands.
const fn caps(p: &Proto) -> Caps {
    Caps {
        sidetone: true,
        mic_mute_status: true,
        battery: true,
        sidetone_level_max: match p.sidetone_level {
            Some((_, _, max)) => max,
            None => 0,
        },
        sidetone_level_zero: p.level_zero,
        auto_off: if p.auto_off.is_some() { p.auto_off_minutes } else { &[] },
        voice_prompts: p.voice_prompts.is_some(),
        ..Caps::NONE
    }
}

/// What changes between the two protocols.
struct Proto {
    /// Leading bytes of request and reply (the first one is the report ID).
    header: &'static [u8],
    /// Reply byte with the battery in %.
    battery_at: usize,
    /// The battery reply carries a reading (the `66` set gives zero when it has none).
    battery_valid: fn(&[u8]) -> bool,
    state: u8,
    battery: u8,
    charging: u8,
    mic_mute: u8,
    sidetone_get: u8,
    sidetone_set: u8,
    /// Mic monitoring volume (get, set, slider maximum); None = not available.
    sidetone_level: Option<(u8, u8, u8)>,
    /// Slider value that is the headset's 0: the `66` set uses -5..+5 (signed, two's
    /// complement), the slider 0..10.
    level_zero: u8,
    /// Auto-off and voice prompts (get, set); None = not available.
    auto_off: Option<(u8, u8)>,
    /// Auto-off minutes offered.
    auto_off_minutes: &'static [u8],
    voice_prompts: Option<(u8, u8)>,
    /// Connection state value meaning headset on and paired.
    connected: fn(u8) -> bool,
    /// Wait after each command (the headset may not accept another one right away).
    write_wait: Duration,
}

impl Proto {
    fn command_at(&self) -> usize {
        self.header.len()
    }

    fn value_at(&self) -> usize {
        self.header.len() + 1
    }

    /// Is the reply to `command`: same header and same command.
    fn is_reply(&self, command: u8, r: &[u8]) -> bool {
        r.starts_with(self.header) && r.get(self.command_at()) == Some(&command)
    }

    /// Battery % from a reply, if it makes sense.
    fn battery_of(&self, r: &[u8]) -> Option<u8> {
        r.get(self.battery_at).copied().filter(|&b| b <= 100 && (self.battery_valid)(r))
    }

    /// Byte sent for a volume slider value, and the reverse (None if out of range).
    fn level_byte(&self, level: u8) -> u8 {
        level.wrapping_sub(self.level_zero)
    }

    fn level_of(&self, byte: u8, max: u8) -> Option<u8> {
        Some(byte.wrapping_add(self.level_zero)).filter(|&v| v <= max)
    }
}

/// Always carries a reading.
const ANY_BATTERY: fn(&[u8]) -> bool = |_| true;

/// Cloud Alpha Wireless: `21 BB <cmd> <value>`.
const ALPHA_WIRELESS: Proto = Proto {
    header: &[0x21, 0xBB],
    battery_at: 3,
    battery_valid: ANY_BATTERY,
    state: 0x03,
    battery: 0x0B,
    charging: 0x0C,
    mic_mute: 0x0A,
    sidetone_get: 0x05,
    sidetone_set: 0x10,
    // From 0 to 128, like HeadsetControl
    sidetone_level: Some((0x06, 0x11, 128)),
    level_zero: 0,
    auto_off: Some((0x07, 0x12)),
    auto_off_minutes: AUTO_OFF,
    voice_prompts: Some((0x09, 0x13)),
    connected: |v| v == 2,
    write_wait: Duration::ZERO,
};

/// Cloud II Wireless DTS: `06 FF BB <cmd> <value>`.
const CLOUD_II_DTS: Proto = Proto {
    header: &[0x06, 0xFF, 0xBB],
    battery_at: 7,
    battery_valid: ANY_BATTERY,
    state: 0x01,
    battery: 0x02,
    charging: 0x03,
    mic_mute: 0x05,
    sidetone_get: 0x06,
    sidetone_set: 0x21,
    sidetone_level: None,
    level_zero: 0,
    auto_off: Some((0x07, 0x22)),
    auto_off_minutes: AUTO_OFF,
    voice_prompts: None,
    connected: |v| v == 1 || v == 4,
    // Wait after writing (as HeadsetControl does)
    write_wait: Duration::from_millis(100),
};

/// Cloud III Wireless and Cloud II Core Wireless: `66 <cmd> <value>`; read requests have bit 7
/// set and the reply repeats the command. The headset also sends notifications on its own (09-0D).
const CLOUD_66: Proto = Proto {
    header: &[0x66],
    // Battery: bytes 2-3 (voltage?) and the % in byte 4; if 2-3 are zero, there is no reading
    battery_at: 4,
    battery_valid: |r| r.get(2..4).is_some_and(|v| v != [0, 0]),
    state: 0x82,
    battery: 0x89,
    charging: 0x8A,
    mic_mute: 0x86,
    sidetone_get: 0x84,
    sidetone_set: 0x01,
    // From -5 to +5 on the headset, 0 by default (used on the Cloud II Core)
    sidetone_level: Some((0x88, 0x05, 10)),
    level_zero: 5,
    auto_off: Some((0x85, 0x02)),
    auto_off_minutes: AUTO_OFF_66,
    voice_prompts: None,
    connected: |v| v == 1,
    write_wait: Duration::from_millis(10),
};

pub const CLOUD_ALPHA_WIRELESS: Model = Model {
    name: "HyperX Cloud Alpha Wireless",
    vid: 0x0951,
    pid: 0x1743,
    interface: 3,
    report_len: 31,
    usage_page: 0,
    collection: 0,
    feature_reports: false,
    caps: caps(&ALPHA_WIRELESS),
    tested: false,
    connect: |model, link| Wireless::boxed(model, link, &ALPHA_WIRELESS, true),
};
pub const CLOUD_ALPHA_WIRELESS_2: Model = Model { pid: 0x1765, ..CLOUD_ALPHA_WIRELESS };
pub const CLOUD_ALPHA_WIRELESS_HP: Model = Model { vid: 0x03F0, pid: 0x098D, ..CLOUD_ALPHA_WIRELESS };

/// Cloud II Wireless DTS USB receiver (with it, the headset may be off).
pub const CLOUD_II_WIRELESS_DTS: Model = Model {
    name: "HyperX Cloud II Wireless DTS",
    vid: 0x03F0,
    pid: 0x018B,
    interface: 3,
    report_len: 52,
    // MI_03&Col03; HeadsetControl selects it by its usage page 0xFF90. Requiring it avoids opening
    // another vendor collection of the same interface
    usage_page: 0xFF90,
    collection: 0,
    feature_reports: false,
    caps: caps(&CLOUD_II_DTS),
    tested: false,
    connect: |model, link| Wireless::boxed(model, link, &CLOUD_II_DTS, true),
};
/// The other IDs of the same headset. Only the 018B receiver is asked for the connection state:
/// these are assumed connected.
pub const CLOUD_II_WIRELESS_DTS_2: Model = Model {
    pid: 0x0696,
    connect: |model, link| Wireless::boxed(model, link, &CLOUD_II_DTS, false),
    ..CLOUD_II_WIRELESS_DTS
};
pub const CLOUD_II_WIRELESS_DTS_3: Model = Model { pid: 0x038B, interface: 0, ..CLOUD_II_WIRELESS_DTS_2 };
pub const CLOUD_II_WIRELESS_DTS_4: Model = Model { pid: 0x058B, interface: 0, ..CLOUD_II_WIRELESS_DTS_2 };

/// Cloud Stinger 2 Wireless: the DTS protocol byte by byte, on mi_03&col03. Its mic state is not
/// read (the `05` reply format is unknown), so it is not shown.
pub const CLOUD_STINGER_2_WIRELESS: Model = Model {
    name: "HyperX Cloud Stinger 2 Wireless",
    pid: 0x0D93,
    usage_page: 0,
    collection: 3,
    caps: Caps { mic_mute_status: false, ..caps(&CLOUD_II_DTS) },
    ..CLOUD_II_WIRELESS_DTS
};

/// Cloud III Wireless: receivers 0C9D and 05B7 (mi_03&col01); 0E9D and 06B7, whole device.
pub const CLOUD_III_WIRELESS: Model = Model {
    name: "HyperX Cloud III Wireless",
    vid: 0x03F0,
    pid: 0x0C9D,
    interface: 3,
    report_len: 62,
    usage_page: 0,
    collection: 1,
    feature_reports: false,
    caps: caps(&CLOUD_66),
    tested: false,
    connect: |model, link| Wireless::boxed(model, link, &CLOUD_66, true),
};
pub const CLOUD_III_WIRELESS_2: Model = Model { pid: 0x05B7, ..CLOUD_III_WIRELESS };
pub const CLOUD_III_WIRELESS_3: Model = Model { pid: 0x0E9D, interface: 0, collection: 0, ..CLOUD_III_WIRELESS };
pub const CLOUD_III_WIRELESS_4: Model = Model { pid: 0x06B7, interface: 0, collection: 0, ..CLOUD_III_WIRELESS };

/// Cloud II Core Wireless (mi_03&col02); 069F is another edition.
pub const CLOUD_II_CORE_WIRELESS: Model =
    Model { name: "HyperX Cloud II Core Wireless", pid: 0x0995, collection: 2, ..CLOUD_III_WIRELESS };
pub const CLOUD_II_CORE_WIRELESS_2: Model = Model { pid: 0x069F, ..CLOUD_II_CORE_WIRELESS };

/// Timeout for each reply (HeadsetControl uses 1-2 s).
const REPLY_TIMEOUT: Duration = Duration::from_millis(1000);
/// Battery and charging change slowly: they are asked at most every 30 s.
const BATTERY_EVERY: Duration = Duration::from_secs(30);
/// Headset card values almost always change from the app (which records what it sets): they are
/// asked every minute, so the state read does not take longer (each waits up to 1 s if the
/// headset does not reply). A change made on the headset itself shows up within a minute.
const CARD_EVERY: Duration = Duration::from_secs(60);

/// Mic monitoring volume, auto-off and voice prompts; None = unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Card {
    level: Option<u8>,
    auto_off: Option<u8>,
    voice_prompts: Option<bool>,
}

impl Card {
    /// From the values read: whatever makes no sense (above the maximum, prompts other than 0 or
    /// 1) stays unknown.
    fn parse(level_max: u8, level: Option<u8>, auto_off: Option<u8>, voice_prompts: Option<u8>) -> Card {
        Card {
            level: level.filter(|&v| v <= level_max),
            auto_off,
            voice_prompts: voice_prompts.filter(|&v| v <= 1).map(|v| v == 1),
        }
    }
}

struct Wireless {
    model: &'static Model,
    link: HidLink,
    proto: &'static Proto,
    /// It is a receiver: the headset may be off or out of range.
    dongle: bool,
    /// The interrupt endpoint reader could be opened (without it no reply arrives).
    reader: bool,
    /// Last battery and charging, and when they were asked.
    battery: Every<(Option<u8>, Option<bool>)>,
    /// Last headset card values, and when they were asked.
    card: Every<Card>,
}

impl Wireless {
    fn boxed(model: &'static Model, link: HidLink, proto: &'static Proto, dongle: bool) -> Box<dyn Headset> {
        // Replies arrive on the interrupt endpoint: start receiving right away
        let reader = link.start_reader();
        let (battery, card) = (Every::new(BATTERY_EVERY), Every::new(CARD_EVERY));
        Box::new(Wireless { model, link, proto, dongle, reader, battery, card })
    }

    fn request(&self, command: u8, value: Option<u8>) -> Vec<u8> {
        let p = self.proto;
        let mut r = self.link.report(p.header[0]);
        r[..p.header.len()].copy_from_slice(p.header);
        r[p.command_at()] = command;
        if let Some(v) = value {
            r[p.value_at()] = v;
        }
        r
    }

    /// Sends a request and waits for its reply (same header and same command).
    fn query(&self, command: u8) -> Res<Vec<u8>> {
        if !self.reader {
            return Err(HidError(t().no_status));
        }
        let p = self.proto;
        let _lock = self.link.lock()?;
        self.link.drain_reader();
        self.link.send(&self.request(command, None), p.header[0])?;
        self.link.read_reply(REPLY_TIMEOUT, |r| p.is_reply(command, r)).ok_or(HidError(t().no_status))
    }

    fn value(&self, command: u8) -> Res<u8> {
        let r = self.query(command)?;
        r.get(self.proto.value_at()).copied().ok_or(HidError(t().short_status))
    }

    /// Battery and charging, from the cache if recent. Either may be missing.
    fn battery(&self) -> (Option<u8>, Option<bool>) {
        let p = self.proto;
        let read = || {
            let battery = self.query(p.battery).ok().and_then(|r| p.battery_of(&r));
            Some((battery, self.value(p.charging).ok().map(|v| v == 1)))
        };
        self.battery.get(read).unwrap_or_default()
    }

    /// Headset card values, from the cache if recent. Any of them may be missing.
    fn card(&self) -> Card {
        let p = self.proto;
        let value = |get: Option<u8>| get.and_then(|get| self.value(get).ok());
        let read = || {
            Some(Card::parse(
                self.model.caps.sidetone_level_max,
                value(p.sidetone_level.map(|(get, _, _)| get)).and_then(|b| p.level_of(b, u8::MAX)),
                value(p.auto_off.map(|(get, _)| get)),
                value(p.voice_prompts.map(|(get, _)| get)),
            ))
        };
        self.card.get(read).unwrap_or_default()
    }

    /// Sends a set command without waiting for a reply.
    fn write(&self, set: Option<u8>, value: u8) -> Res<()> {
        let Some(set) = set else { return Err(HidError("unsupported")) };
        let _lock = self.link.lock()?;
        self.link.drain_reader();
        self.link.send(&self.request(set, Some(value)), self.proto.header[0])?;
        std::thread::sleep(self.proto.write_wait);
        Ok(())
    }

    /// Sets a card value if it is not already set (like mic monitoring: settings are reapplied on
    /// connect and power-on) and records it: reading it right after could still return the
    /// previous one.
    /// `byte` is `value` as sent.
    fn set_card<T: Copy + PartialEq>(&self, set: Option<u8>, value: T, byte: u8, field: fn(&mut Card) -> &mut Option<T>) -> Res<()> {
        let mut card = self.card();
        if *field(&mut card) != Some(value) {
            self.write(set, byte)?;
            *field(&mut card) = Some(value);
            self.card.set(card);
        }
        Ok(())
    }
}

impl Headset for Wireless {
    fn model(&self) -> &'static Model {
        self.model
    }

    fn firmware(&self) -> u16 {
        self.link.firmware
    }

    /// Each value the headset does not answer is left empty instead of failing the whole state
    /// (these are untested commands); only if nothing is answered is it an error.
    fn status(&self) -> Res<Status> {
        let p = self.proto;
        // If the receiver says nothing about the state (or is not a receiver), carry on with the rest
        let state = if self.dongle { self.value(p.state).ok() } else { None };
        if state.is_some_and(|v| !(p.connected)(v)) {
            // The receiver replies but the headset is off or far away
            self.battery.clear();
            self.card.clear();
            return Ok(Status { asleep: true, ..Status::default() });
        }
        let sidetone = self.value(p.sidetone_get).ok().map(|v| v == 1);
        // Only if the model shows it: each unanswered query waits up to 1 s
        let mic_muted = if self.model.caps.mic_mute_status { self.value(p.mic_mute).ok().map(|v| v == 1) } else { None };
        let (battery, charging) = self.battery();
        if state.is_none() && sidetone.is_none() && mic_muted.is_none() && battery.is_none() && charging.is_none() {
            return Err(HidError(t().no_status));
        }
        let card = self.card();
        Ok(Status {
            sidetone: sidetone.unwrap_or(false),
            mic_muted: mic_muted.unwrap_or(false),
            battery,
            charging: charging.unwrap_or(false),
            sidetone_level: card.level,
            auto_off: card.auto_off,
            voice_prompts: card.voice_prompts,
            ..Status::default()
        })
    }

    /// Only the volume: mic monitoring is turned on and off with its switch (and turning it on
    /// sends the volume again).
    fn set_sidetone_level(&self, level: u8) -> Res<()> {
        let level = level.min(self.model.caps.sidetone_level_max);
        let p = self.proto;
        self.set_card(p.sidetone_level.map(|(_, set, _)| set), level, p.level_byte(level), |c| &mut c.level)
    }

    fn set_auto_off(&self, minutes: u8) -> Res<()> {
        self.set_card(self.proto.auto_off.map(|(_, set)| set), minutes, minutes, |c| &mut c.auto_off)
    }

    fn set_voice_prompts(&self, on: bool) -> Res<()> {
        self.set_card(self.proto.voice_prompts.map(|(_, set)| set), on, on as u8, |c| &mut c.voice_prompts)
    }

    /// Sets mic monitoring (it is a value, it does not toggle). No reply is awaited.
    fn set_sidetone(&self, on: bool) -> Res<bool> {
        if self.status()?.sidetone == on {
            return Ok(false);
        }
        self.write(Some(self.proto.sidetone_set), on as u8)?;
        // Like HeadsetControl: when turning it on, the volume (it may have changed while off)
        if let (true, Some((_, set, _)), Some(level)) = (on, self.proto.sidetone_level, self.card().level) {
            self.write(Some(set), self.proto.level_byte(level))?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_layouts() {
        assert_eq!((ALPHA_WIRELESS.command_at(), ALPHA_WIRELESS.value_at()), (2, 3));
        assert_eq!((CLOUD_II_DTS.command_at(), CLOUD_II_DTS.value_at()), (3, 4));
        assert!((ALPHA_WIRELESS.connected)(2) && !(ALPHA_WIRELESS.connected)(1));
        assert!((CLOUD_II_DTS.connected)(1) && (CLOUD_II_DTS.connected)(4) && !(CLOUD_II_DTS.connected)(2));
    }

    #[test]
    fn caps_follow_the_commands() {
        let alpha = CLOUD_ALPHA_WIRELESS_HP.caps;
        assert_eq!((alpha.sidetone_level_max, alpha.auto_off, alpha.voice_prompts), (128, AUTO_OFF, true));
        // The DTS: only auto-off
        let dts = CLOUD_II_WIRELESS_DTS_4.caps;
        assert_eq!((dts.sidetone_level_max, dts.auto_off, dts.voice_prompts), (0, AUTO_OFF, false));
        assert!(alpha.sidetone && alpha.battery && dts.mic_mute_status);
    }

    #[test]
    fn cloud_66() {
        // Volume: slider 0..10 is -5..+5 in two's complement
        assert_eq!((CLOUD_66.level_byte(0), CLOUD_66.level_byte(5), CLOUD_66.level_byte(10)), (0xFB, 0x00, 0x05));
        assert_eq!((CLOUD_66.level_of(0xFB, 10), CLOUD_66.level_of(0x05, 10), CLOUD_66.level_of(0x06, 10)), (Some(0), Some(10), None));
        // The Alpha Wireless is unchanged: 0..128 as is
        assert_eq!((ALPHA_WIRELESS.level_byte(128), ALPHA_WIRELESS.level_of(128, 128)), (128, Some(128)));
        // Battery: 66 89 <2 bytes> <%>, no reading if both bytes are zero
        let r = [0x66, 0x89, 0x0F, 0xA0, 72];
        assert!(CLOUD_66.is_reply(0x89, &r) && !CLOUD_66.is_reply(0x8A, &r));
        assert_eq!(CLOUD_66.battery_of(&r), Some(72));
        assert_eq!(CLOUD_66.battery_of(&[0x66, 0x89, 0, 0, 72]), None);
        assert_eq!((CLOUD_66.command_at(), CLOUD_66.value_at()), (1, 2));
        let c = CLOUD_II_CORE_WIRELESS_2.caps;
        assert_eq!((c.sidetone_level_max, c.auto_off, c.voice_prompts), (10, AUTO_OFF_66, false));
        // Stinger 2: the DTS protocol without the mic state
        let s = CLOUD_STINGER_2_WIRELESS.caps;
        assert!(!s.mic_mute_status && s.battery && s.sidetone && s.auto_off == AUTO_OFF);
    }

    #[test]
    fn card_values() {
        let c = Card::parse(128, Some(64), Some(20), Some(1));
        assert_eq!(c, Card { level: Some(64), auto_off: Some(20), voice_prompts: Some(true) });
        // Out of range: unknown
        let c = Card::parse(128, Some(200), None, Some(7));
        assert_eq!(c, Card { level: None, auto_off: None, voice_prompts: None });
        assert_eq!(Card::parse(0, Some(1), None, Some(0)).voice_prompts, Some(false));
    }

    #[test]
    fn replies_and_battery() {
        // Alpha Wireless: 21 BB 0B <battery>
        assert!(ALPHA_WIRELESS.is_reply(0x0B, &[0x21, 0xBB, 0x0B, 87]));
        assert!(!ALPHA_WIRELESS.is_reply(0x0B, &[0x21, 0xBB, 0x0C, 1]));
        assert!(!ALPHA_WIRELESS.is_reply(0x0B, &[0x06, 0xFF, 0xBB, 0x0B]));
        assert_eq!(ALPHA_WIRELESS.battery_of(&[0x21, 0xBB, 0x0B, 87]), Some(87));
        // DTS: 06 FF BB 02 .. .. .. <battery in byte 7>
        let r = [0x06, 0xFF, 0xBB, 0x02, 0, 0x0F, 0xA0, 64];
        assert!(CLOUD_II_DTS.is_reply(0x02, &r));
        assert_eq!(CLOUD_II_DTS.battery_of(&r), Some(64));
        assert_eq!(CLOUD_II_DTS.battery_of(&[0x06, 0xFF, 0xBB, 0x02, 0, 0, 0, 200]), None);
        assert_eq!(CLOUD_II_DTS.battery_of(&[0x06, 0xFF, 0xBB, 0x02]), None);
    }
}
