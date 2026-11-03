//! Headset models. Each protocol family is a module that declares its USB IDs and its
//! capabilities and implements `Headset`; the UI only shows what the connected model can do
//! (see `layout.rs`). Adding a model = a module + its entry in `MODELS`.

mod alpha_s;
mod basic;
mod classic;
mod cloud3_wired;
mod demo;
mod every;
mod wireless;

use crate::hid::{HidError, HidLink, Res};

/// EQ band: (Hz, dB, Q, filter type: 0 peaking, 1 low shelf)
pub type Band = (f32, f32, f32, u8);

/// What a model can do. Whatever it lacks, the UI does not show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Caps {
    /// Virtual 7.1: turn on and read its state.
    pub surround: bool,
    /// Mic monitoring: turn on and read its state.
    pub sidetone: bool,
    /// The state says whether the mic is muted (muted on the headset or box itself).
    pub mic_mute_status: bool,
    /// Parametric EQ bands (0 = no EQ).
    pub eq_bands: u8,
    /// The EQ is only audible with 7.1 on (it runs inside its DSP).
    pub eq_needs_surround: bool,
    /// The state includes the control box game and chat levels.
    pub box_levels: bool,
    /// Separate "Game" and "Chat" audio outputs in the system, with this name: balance is done
    /// with their volume.
    pub game_chat_outputs: Option<&'static str>,
    /// The state includes the battery.
    pub battery: bool,
    /// Mic monitoring volume: maximum value the headset accepts (0 = not adjustable).
    pub sidetone_level_max: u8,
    /// Slider value that is the headset's 0, if its scale is signed (0 = from 0 to max).
    pub sidetone_level_zero: u8,
    /// Auto-off: selectable minutes (0 = never); empty = not adjustable.
    pub auto_off: &'static [u8],
    /// Voice prompts (power, battery, mic).
    pub voice_prompts: bool,
}

impl Caps {
    pub const NONE: Caps = Caps {
        surround: false,
        sidetone: false,
        mic_mute_status: false,
        eq_bands: 0,
        eq_needs_surround: false,
        box_levels: false,
        game_chat_outputs: None,
        battery: false,
        sidetone_level_max: 0,
        sidetone_level_zero: 0,
        auto_off: &[],
        voice_prompts: false,
    };
}

pub struct Model {
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
    /// USB interface of the control HID collection.
    pub interface: u8,
    /// Report size, for when the system does not report it (Linux).
    pub report_len: usize,
    /// HID usage page of the control collection, if it must be required (0 = any vendor page,
    /// 0xFF00 or above). Only Windows distinguishes collections within an interface, so a model
    /// with a page below that range needs it here to be found at all.
    pub usage_page: u16,
    /// Number of the control collection (`ColNN` in the Windows path) if it must be required;
    /// 0 = the first one that matches.
    pub collection: u8,
    /// The model is controlled with feature reports (on Windows its collection may have no output).
    pub feature_reports: bool,
    pub caps: Caps,
    /// Tested on real hardware; otherwise the UI marks it as experimental.
    pub tested: bool,
    connect: fn(&'static Model, HidLink) -> Box<dyn Headset>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    pub surround: bool,
    pub sidetone: bool,
    pub mic_muted: bool,
    pub game_db: f32,
    pub chat_db: f32,
    /// Battery percentage, if the model reports it.
    pub battery: Option<u8>,
    /// Charging.
    pub charging: bool,
    /// The receiver replies but the headset is off or out of range.
    pub asleep: bool,
    /// Mic monitoring volume (0..=Caps::sidetone_level_max), if the model reports it.
    pub sidetone_level: Option<u8>,
    /// Auto-off in minutes (0 = never), if the model reports it.
    pub auto_off: Option<u8>,
    /// Voice prompts, if the model reports them.
    pub voice_prompts: Option<bool>,
}

/// Change made on the headset itself or on its box (a button).
#[derive(Clone, Copy, Debug)]
pub struct ButtonEvent {
    pub surround: bool,
    pub game_db: f32,
    pub chat_db: f32,
}

fn unsupported<T>() -> Res<T> {
    Err(HidError("unsupported"))
}

/// A model's protocol. Whatever the model lacks fails by default: the UI does not ask for it
/// because it checks its `Caps` first.
pub trait Headset: Send {
    fn model(&self) -> &'static Model;
    fn firmware(&self) -> u16;
    fn status(&self) -> Res<Status>;

    /// Sets 7.1. Returns true if it had to be changed.
    fn set_surround(&self, _on: bool) -> Res<bool> {
        unsupported()
    }

    /// Sets mic monitoring. Returns true if it had to be changed.
    fn set_sidetone(&self, _on: bool) -> Res<bool> {
        unsupported()
    }

    /// Sets the mic monitoring volume (0..=Caps::sidetone_level_max).
    fn set_sidetone_level(&self, _level: u8) -> Res<()> {
        unsupported()
    }

    /// Sets auto-off in minutes (0 = never), one of Caps::auto_off.
    fn set_auto_off(&self, _minutes: u8) -> Res<()> {
        unsupported()
    }

    /// Turns voice prompts on or off.
    fn set_voice_prompts(&self, _on: bool) -> Res<()> {
        unsupported()
    }

    /// Sends one EQ band. True if the headset confirmed it.
    fn eq_band(&self, _index: usize, _band: Band) -> Res<bool> {
        unsupported()
    }

    /// Sends the whole EQ, up to the bands the model supports.
    fn eq(&self, bands: &[Band]) -> Res<bool> {
        let mut ok = true;
        for (i, b) in bands.iter().take(self.model().caps.eq_bands as usize).enumerate() {
            ok &= self.eq_band(i, *b)?;
        }
        Ok(ok)
    }

    /// Blocks until the next button. None if disconnected or the model does not report them.
    fn wait_button(&self) -> Option<ButtonEvent> {
        None
    }
}

pub const MODELS: &[&Model] = &[
    &alpha_s::MODEL,
    &alpha_s::MODEL_HP,
    &classic::FLIGHT_S,
    &classic::CLOUD_II_WIRELESS,
    &classic::CLOUD_II_WIRELESS_HP,
    &wireless::CLOUD_ALPHA_WIRELESS,
    &wireless::CLOUD_ALPHA_WIRELESS_2,
    &wireless::CLOUD_ALPHA_WIRELESS_HP,
    &wireless::CLOUD_II_WIRELESS_DTS,
    &wireless::CLOUD_II_WIRELESS_DTS_2,
    &wireless::CLOUD_II_WIRELESS_DTS_3,
    &wireless::CLOUD_II_WIRELESS_DTS_4,
    &basic::CLOUD_FLIGHT_WIRELESS,
    &basic::CLOUD_FLIGHT_WIRELESS_2,
    &basic::CLOUD_FLIGHT_WIRELESS_HP,
    &basic::CLOUD_STINGER_CORE_WIRELESS,
    &wireless::CLOUD_STINGER_2_WIRELESS,
    &wireless::CLOUD_III_WIRELESS,
    &wireless::CLOUD_III_WIRELESS_2,
    &wireless::CLOUD_III_WIRELESS_3,
    &wireless::CLOUD_III_WIRELESS_4,
    &wireless::CLOUD_II_CORE_WIRELESS,
    &wireless::CLOUD_II_CORE_WIRELESS_2,
    &cloud3_wired::CLOUD_III,
    &cloud3_wired::CLOUD_III_2,
    &cloud3_wired::CLOUD_III_3,
];

/// Model by its name (the one stored in the settings).
pub fn find(name: &str) -> Option<&'static Model> {
    MODELS.iter().copied().chain(demo::MODELS.iter().copied()).find(|m| m.name == name)
}

/// Simulated headset to test the UI without hardware: NOGENUITY_DEMO=1 (every capability),
/// NOGENUITY_DEMO=basic (only mic and battery) or NOGENUITY_DEMO=wireless (mic, battery and the
/// headset card).
fn demo_model() -> Option<&'static Model> {
    match std::env::var("NOGENUITY_DEMO").ok()?.as_str() {
        "1" => Some(&demo::FULL),
        "basic" => Some(&demo::BASIC),
        "wireless" => Some(&demo::WIRELESS),
        _ => None,
    }
}

pub fn demo_mode() -> bool {
    demo_model().is_some()
}

/// Opens the first known headset that is connected.
pub fn open() -> Option<Box<dyn Headset>> {
    crate::sys::begin_scan();
    if let Some(model) = demo_model() {
        return Some(demo::open(model));
    }
    MODELS.iter().find_map(|m| open_model(m))
}

/// Opens another connection to a specific model (the buttons thread uses its own).
pub fn open_model(model: &'static Model) -> Option<Box<dyn Headset>> {
    if demo::MODELS.iter().any(|m| std::ptr::eq(model, *m)) {
        return Some(demo::open(model));
    }
    let link = HidLink::open(
        model.vid,
        model.pid,
        model.interface,
        model.usage_page,
        model.collection,
        model.feature_reports,
        model.report_len,
    )?;
    Some((model.connect)(model, link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_have_unique_ids() {
        for (i, a) in MODELS.iter().enumerate() {
            for b in &MODELS[i + 1..] {
                assert!((a.vid, a.pid) != (b.vid, b.pid), "{} and {} share an ID", a.name, b.name);
            }
        }
    }

    #[test]
    fn eq_needs_surround_implies_both() {
        for m in MODELS {
            if m.caps.eq_needs_surround {
                assert!(m.caps.surround && m.caps.eq_bands > 0, "{}", m.name);
            }
        }
    }
}
