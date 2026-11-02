//! Settings in settings.txt inside the data folder (key=value, no dependencies).
//! Windows: %APPDATA%\Nogenuity. Linux: ~/.config/nogenuity.

use std::path::PathBuf;

use crate::backend::OnConnect;
use crate::eq::PRESETS;
use crate::devices::{Band, Status};
use crate::layout::Layout;
pub use crate::sys::{autostart_enabled, set_autostart};

#[derive(Clone, Debug)]
pub struct Settings {
    pub eq: Vec<Band>,
    pub preset: Option<usize>,
    /// -100 = all game, 0 = center, +100 = all chat
    pub balance: i32,
    /// System volume of the Game and Chat outputs when the balance was centered
    pub balance_base: Option<(f32, f32)>,
    /// Last 7.1 state (None = never touched: not forced on connect)
    pub surround: Option<bool>,
    /// Last mic monitoring state; on first run it is turned on
    pub sidetone: Option<bool>,
    /// Last headset card values (None = unknown: not forced on connect).
    /// The headset reverts to factory values on power-on, so they are reapplied.
    pub sidetone_level: Option<u8>,
    pub auto_off: Option<u8>,
    pub voice_prompts: Option<bool>,
    /// Last connected model: the window shows its cards before it connects
    pub model: Option<String>,
    /// true if the file already existed (to know whether this is the first launch)
    pub existed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            eq: PRESETS[0].to_vec(),
            preset: Some(0),
            balance: 0,
            balance_base: None,
            surround: None,
            sidetone: Some(true),
            sidetone_level: None,
            auto_off: None,
            voice_prompts: None,
            model: None,
            existed: false,
        }
    }
}

fn path() -> Option<PathBuf> {
    // The simulated headset doesn't touch the real settings
    let name = if crate::devices::demo_mode() { "settings-demo.txt" } else { "settings.txt" };
    Some(crate::sys::data_dir()?.join(name))
}

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return s;
        };
        s.existed = true;
        s.preset = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim() {
                "eq" => {
                    let bands: Vec<Band> = value
                        .split(';')
                        .filter_map(|b| {
                            let v: Vec<f32> = b.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                            (v.len() == 4).then(|| (v[0], v[1], v[2], v[3] as u8))
                        })
                        .collect();
                    if bands.len() == 10 {
                        s.eq = bands;
                    }
                }
                "preset" => s.preset = value.trim().parse().ok(),
                "balance" => s.balance = value.trim().parse().unwrap_or(0),
                "balance_base" => {
                    s.balance_base = value.split_once(',').and_then(|(g, c)| Some((g.trim().parse().ok()?, c.trim().parse().ok()?)))
                }
                "surround" => s.surround = Some(value.trim() == "1"),
                "sidetone" => s.sidetone = Some(value.trim() == "1"),
                "sidetone_level" => s.sidetone_level = value.trim().parse().ok(),
                "auto_off" => s.auto_off = value.trim().parse().ok(),
                "voice_prompts" => s.voice_prompts = Some(value.trim() == "1"),
                "model" => s.model = Some(value.trim().to_string()),
                _ => {}
            }
        }
        // Only mark the preset if the bands are exactly its own (tweaked EQ = custom)
        if s.preset.is_some_and(|p| PRESETS.get(p).is_none_or(|bands| bands[..] != s.eq[..])) {
            s.preset = None;
        }
        s
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let eq: Vec<String> = self.eq.iter().map(|(f, g, q, t)| format!("{f},{g},{q},{t}")).collect();
        let mut text = format!("eq={}\nbalance={}\n", eq.join(";"), self.balance);
        if let Some(on) = self.surround {
            text += &format!("surround={}\n", on as u8);
        }
        if let Some(on) = self.sidetone {
            text += &format!("sidetone={}\n", on as u8);
        }
        if let Some(level) = self.sidetone_level {
            text += &format!("sidetone_level={level}\n");
        }
        if let Some(minutes) = self.auto_off {
            text += &format!("auto_off={minutes}\n");
        }
        if let Some(on) = self.voice_prompts {
            text += &format!("voice_prompts={}\n", on as u8);
        }
        if let Some(p) = self.preset {
            text += &format!("preset={p}\n");
        }
        if let Some(m) = &self.model {
            text += &format!("model={m}\n");
        }
        if let Some((g, c)) = self.balance_base {
            text += &format!("balance_base={g},{c}\n");
        }
        let _ = std::fs::write(p, text);
    }

    /// What is applied when the headset connects or powers on.
    pub fn on_connect(&self) -> OnConnect {
        OnConnect {
            eq: self.eq.clone(),
            surround: self.surround,
            sidetone: self.sidetone,
            sidetone_level: self.sidetone_level,
            auto_off: self.auto_off,
            voice_prompts: self.voice_prompts,
            model: self.model.clone(),
        }
    }

    /// Records the connected model. If it's a different one, the headset card values belonged to the
    /// previous one (another scale, other options): they're forgotten. True if it changed.
    pub fn set_model(&mut self, name: &str) -> bool {
        if self.model.as_deref() == Some(name) {
            return false;
        }
        self.model = Some(name.to_string());
        (self.sidetone_level, self.auto_off, self.voice_prompts) = (None, None, None);
        true
    }

    /// Remembers the state reported by the headset, only for what the model has (otherwise its state
    /// is always "off") and what it reports (a value it doesn't read doesn't erase the last known one).
    /// With the headset off, the state says nothing about its settings. True if anything changed.
    pub fn remember(&mut self, l: &Layout, s: &Status) -> bool {
        if s.asleep {
            return false;
        }
        let before = (self.surround, self.sidetone, self.sidetone_level, self.auto_off, self.voice_prompts);
        if l.surround {
            self.surround = Some(s.surround);
        }
        if l.sidetone {
            self.sidetone = Some(s.sidetone);
        }
        if l.sidetone_level {
            self.sidetone_level = s.sidetone_level.or(self.sidetone_level);
        }
        if l.auto_off {
            self.auto_off = s.auto_off.or(self.auto_off);
        }
        if l.voice_prompts {
            self.voice_prompts = s.voice_prompts.or(self.voice_prompts);
        }
        before != (self.surround, self.sidetone, self.sidetone_level, self.auto_off, self.voice_prompts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::Caps;

    #[test]
    fn remembers_only_what_the_headset_reports() {
        let wireless = Layout::for_caps(&Caps { sidetone: true, sidetone_level_max: 128, voice_prompts: true, ..Caps::NONE });
        let mut set = Settings { sidetone_level: Some(40), voice_prompts: Some(false), ..Settings::default() };
        // Reads the volume but not the voice prompts: the volume is updated, the prompts are kept
        let s = Status { sidetone: true, sidetone_level: Some(90), auto_off: Some(30), ..Status::default() };
        assert!(set.remember(&wireless, &s));
        assert_eq!((set.sidetone_level, set.voice_prompts), (Some(90), Some(false)));
        // The model has no auto power-off or 7.1: not remembered
        assert_eq!((set.auto_off, set.surround), (None, None));
        assert!(!set.remember(&wireless, &s));
        // Off: nothing
        assert!(!set.remember(&wireless, &Status { asleep: true, ..Status::default() }));
        assert_eq!(set.sidetone_level, Some(90));
    }

    #[test]
    fn another_model_forgets_the_card() {
        let mut set = Settings { model: Some("A".into()), sidetone_level: Some(64), auto_off: Some(60), ..Settings::default() };
        assert!(!set.set_model("A"));
        assert_eq!(set.sidetone_level, Some(64));
        assert!(set.set_model("B"));
        assert_eq!((set.sidetone_level, set.auto_off, set.on_connect().model), (None, None, Some("B".into())));
    }
}
