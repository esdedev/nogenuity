//! Simulated headsets (NOGENUITY_DEMO): in-memory state, to test the UI and take screenshots
//! without hardware. FULL has every capability; BASIC, only mic and battery; WIRELESS, what a
//! wireless model has (mic, battery and the headset card, no balance or EQ).

use std::sync::Mutex;

use super::{Band, Caps, Headset, Model, Status};
use crate::hid::Res;

pub const FULL: Model = Model {
    name: "Demo headset",
    vid: 0,
    pid: 0,
    interface: 0,
    report_len: 0,
    usage_page: 0,
    collection: 0,
    feature_reports: false,
    caps: Caps {
        surround: true,
        sidetone: true,
        mic_mute_status: true,
        eq_bands: 10,
        eq_needs_surround: false,
        box_levels: true,
        game_chat_outputs: None,
        battery: true,
        sidetone_level_max: 100,
        sidetone_level_zero: 0,
        auto_off: &[0, 10, 20, 30, 60],
        voice_prompts: true,
    },
    tested: false,
    connect: |_, _| unreachable!("the demo does not use HID"),
};

pub const BASIC: Model = Model {
    name: "Demo basic headset",
    caps: Caps { sidetone: true, mic_mute_status: true, battery: true, ..Caps::NONE },
    ..FULL
};

pub const WIRELESS: Model = Model {
    name: "Demo wireless headset",
    caps: Caps {
        sidetone: true,
        mic_mute_status: true,
        battery: true,
        sidetone_level_max: 100,
        // More options than FULL: they don't fit in full on a half card and use the short form
        auto_off: &[0, 5, 10, 15, 20, 30, 60],
        voice_prompts: true,
        ..Caps::NONE
    },
    ..FULL
};

pub const MODELS: &[&Model] = &[&FULL, &BASIC, &WIRELESS];

/// Shared by all connections, as a real headset would be.
static STATE: Mutex<Status> = Mutex::new(Status {
    surround: false,
    sidetone: true,
    mic_muted: false,
    game_db: 0.0,
    chat_db: -6.0,
    battery: Some(80),
    charging: false,
    asleep: false,
    sidetone_level: Some(60),
    auto_off: Some(20),
    voice_prompts: Some(true),
});

struct Demo(&'static Model);

pub fn open(model: &'static Model) -> Box<dyn Headset> {
    Box::new(Demo(model))
}

impl Headset for Demo {
    fn model(&self) -> &'static Model {
        self.0
    }

    fn firmware(&self) -> u16 {
        0
    }

    fn status(&self) -> Res<Status> {
        Ok(*STATE.lock().unwrap())
    }

    fn set_surround(&self, on: bool) -> Res<bool> {
        let mut s = STATE.lock().unwrap();
        Ok(std::mem::replace(&mut s.surround, on) != on)
    }

    fn set_sidetone(&self, on: bool) -> Res<bool> {
        let mut s = STATE.lock().unwrap();
        Ok(std::mem::replace(&mut s.sidetone, on) != on)
    }

    fn set_sidetone_level(&self, level: u8) -> Res<()> {
        STATE.lock().unwrap().sidetone_level = Some(level);
        Ok(())
    }

    fn set_auto_off(&self, minutes: u8) -> Res<()> {
        STATE.lock().unwrap().auto_off = Some(minutes);
        Ok(())
    }

    fn set_voice_prompts(&self, on: bool) -> Res<()> {
        STATE.lock().unwrap().voice_prompts = Some(on);
        Ok(())
    }

    fn eq_band(&self, _index: usize, _band: Band) -> Res<bool> {
        Ok(true)
    }
}
