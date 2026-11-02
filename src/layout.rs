//! Which cards and controls are shown depending on the connected headset's capabilities, and where
//! they go in the window. Pure logic, shared by both UIs and tested here: each UI
//! only draws it.

use crate::devices::{Caps, Status, MODELS};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Game / chat balance card.
    pub balance: bool,
    /// Balance slider (via the system volume of the Game and Chat outputs).
    pub balance_slider: bool,
    /// Game and chat levels of the control box.
    pub box_levels: bool,
    /// Microphone and sound card.
    pub mic: bool,
    /// Mic muted indicator.
    pub mic_status: bool,
    pub sidetone: bool,
    pub surround: bool,
    pub eq: bool,
    /// Notice that the EQ is only audible with 7.1.
    pub eq_needs_surround: bool,
    pub battery: bool,
    /// Headset card rows: mic monitoring volume, auto power-off, voice prompts.
    pub sidetone_level: bool,
    pub auto_off: bool,
    pub voice_prompts: bool,
}

impl Layout {
    pub fn for_caps(c: &Caps) -> Layout {
        let balance_slider = c.game_chat_outputs.is_some();
        let mic_status = c.mic_mute_status;
        Layout {
            balance: balance_slider || c.box_levels,
            balance_slider,
            box_levels: c.box_levels,
            mic: mic_status || c.sidetone || c.surround,
            mic_status,
            sidetone: c.sidetone,
            surround: c.surround,
            eq: c.eq_bands > 0,
            eq_needs_surround: c.eq_needs_surround,
            battery: c.battery,
            sidetone_level: c.sidetone_level_max > 0,
            auto_off: !c.auto_off.is_empty(),
            voice_prompts: c.voice_prompts,
        }
    }

    /// Before any headset connects: whatever any of the known models has, so
    /// the window doesn't come up empty.
    pub fn before_connect() -> Layout {
        let all = MODELS.iter().fold(Caps::default(), |a, m| {
            let c = m.caps;
            Caps {
                surround: a.surround || c.surround,
                sidetone: a.sidetone || c.sidetone,
                mic_mute_status: a.mic_mute_status || c.mic_mute_status,
                eq_bands: a.eq_bands.max(c.eq_bands),
                eq_needs_surround: a.eq_needs_surround || c.eq_needs_surround,
                box_levels: a.box_levels || c.box_levels,
                game_chat_outputs: a.game_chat_outputs.or(c.game_chat_outputs),
                ..Caps::NONE
            }
        });
        Layout::for_caps(&all)
    }

    /// Height of the first settings row of the mic card (below the indicator, if any).
    pub fn mic_rows_top(&self) -> f32 {
        if self.mic_status { 120.0 } else { HEADSET_ROW_TOP }
    }

    /// Height each small card needs: down to its last row. The balance card has a fixed one.
    fn card_heights(&self) -> [f32; 3] {
        let rows = |top: f32, n: usize| (top + n as f32 * HEADSET_ROW_H + 4.0).max(MIN_CARD_H);
        let mic_rows = [self.sidetone, self.surround].iter().filter(|r| **r).count();
        let headset_rows = self.headset_rows().iter().flatten().count();
        [BALANCE_H, rows(self.mic_rows_top(), mic_rows), rows(HEADSET_ROW_TOP, headset_rows)]
    }

    /// Headset card: if it has any of its rows.
    pub fn headset(&self) -> bool {
        self.sidetone_level || self.auto_off || self.voice_prompts
    }

    /// Height of each headset card row within it (volume, power-off, prompts),
    /// only those the model has, top to bottom. Rows are as tall as the mic card's.
    pub fn headset_rows(&self) -> [Option<f32>; 3] {
        let mut y = HEADSET_ROW_TOP;
        [self.sidetone_level, self.auto_off, self.voice_prompts].map(|shown| {
            shown.then(|| {
                y += HEADSET_ROW_H;
                y - HEADSET_ROW_H
            })
        })
    }
}

/// First row of the headset card (below the title) and height of each row.
pub const HEADSET_ROW_TOP: f32 = 56.0;
pub const HEADSET_ROW_H: f32 = 46.0;
/// Height of the balance card (its content doesn't change), and the minimum for any card.
const BALANCE_H: f32 = 216.0;
const MIN_CARD_H: f32 = 120.0;

/// Mic monitoring volume as a fraction of the bar (0..1), if known.
pub fn level_share(level: Option<u8>, max: u8) -> Option<f32> {
    level.map(|v| (v as f32 / max.max(1) as f32).clamp(0.0, 1.0))
}

/// Volume text next to the bar: in %, or, if the headset's scale is signed (its 0 is
/// at `zero`), in steps from that 0, which is the factory value.
pub fn level_text(level: Option<u8>, max: u8, zero: u8) -> String {
    match (level, zero) {
        (None, _) => "—".to_string(),
        (Some(v), 0) => format!("{:.0} %", level_share(Some(v), max).unwrap_or(0.0) * 100.0),
        (Some(v), z) if v == z => "0".to_string(),
        (Some(v), z) => format!("{:+}", v as i16 - z as i16),
    }
}

/// Auto power-off chips in a row `width` DIPs wide: (text, minutes) for each option, and whether
/// they use the short form. If «10 MIN» doesn't fit, only the number (and minutes move to the title).
pub fn auto_off_chips(options: &[u8], width: f32, never: &str, never_short: &str, unit: &str) -> (Vec<(String, u8)>, bool) {
    let n = options.len().max(1) as f32;
    let chip = (width - AUTO_OFF_GAP * (n - 1.0)) / n;
    let short = chip < 52.0;
    let chips = options
        .iter()
        .map(|&m| {
            let label = match (m, short) {
                (0, false) => never.to_string(),
                (0, true) => never_short.to_string(),
                (_, false) => format!("{m} {unit}"),
                (_, true) => m.to_string(),
            };
            (label.to_uppercase(), m)
        })
        .collect();
    (chips, short)
}

/// Spacing between auto power-off chips.
pub const AUTO_OFF_GAP: f32 = 6.0;

/// Connected headset status in the header pill.
#[derive(Clone, Debug, PartialEq)]
pub enum Pill {
    /// Connected; with battery if the model reports it (then without firmware, so it fits)
    Connected { firmware: u16, battery: Option<u8> },
    Charging(Option<u8>),
    /// The receiver is present, the headset isn't
    HeadsetOff,
}

pub fn pill(firmware: u16, status: Option<Status>, layout: &Layout) -> Pill {
    let battery = status.and_then(|s| s.battery).filter(|_| layout.battery);
    match status {
        Some(s) if s.asleep => Pill::HeadsetOff,
        Some(s) if s.charging && layout.battery => Pill::Charging(battery),
        _ => Pill::Connected { firmware, battery },
    }
}

impl Pill {
    pub fn text(&self, connected: &str, charging: &str, off: &str) -> String {
        let with_battery = |label: &str, b: Option<u8>| match b {
            Some(pct) => format!("{label}  ·  {pct} %"),
            None => label.to_string(),
        };
        match *self {
            Pill::Connected { battery: Some(b), .. } => with_battery(connected, Some(b)),
            Pill::Connected { firmware: 0, .. } => connected.to_string(),
            Pill::Connected { firmware, .. } => format!("{connected}  ·  FW {firmware:04X}"),
            Pill::Charging(b) => with_battery(charging, b),
            Pill::HeadsetOff => off.to_string(),
        }
    }
}

/// Rectangle in DIPs: (x0, y0, x1, y1).
pub type Rect = (f32, f32, f32, f32);

/// Position of each card in the window (760 wide, in DIPs / logical px). Cards that aren't
/// shown take no space and the window gets shorter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub balance: Option<Rect>,
    pub mic: Option<Rect>,
    pub headset: Option<Rect>,
    pub eq: Option<Rect>,
    pub footer: Rect,
    pub height: f32,
}

impl Frame {
    pub fn new(l: &Layout) -> Frame {
        const LEFT: f32 = 20.0;
        const RIGHT: f32 = 740.0;
        const GAP: f32 = 16.0;
        const EQ_H: f32 = 346.0;
        let mut y = 84.0;
        // Small cards (balance, mic, headset) two by two, side by side; if one is left
        // alone in its row, it takes the full width. Each row is as tall as its tallest card
        let heights = l.card_heights();
        let mut cards: [Option<Rect>; 3] = [None; 3];
        let shown: Vec<usize> = [l.balance, l.mic, l.headset()].iter().enumerate().filter(|c| *c.1).map(|c| c.0).collect();
        let mid = (LEFT + RIGHT) / 2.0;
        for pair in shown.chunks(2) {
            let h = pair.iter().map(|&c| heights[c]).fold(0.0, f32::max);
            match *pair {
                [a, b] => {
                    cards[a] = Some((LEFT, y, mid - GAP / 2.0, y + h));
                    cards[b] = Some((mid + GAP / 2.0, y, RIGHT, y + h));
                }
                [a] => cards[a] = Some((LEFT, y, RIGHT, y + h)),
                _ => unreachable!(),
            }
            y += h + GAP;
        }
        let [balance, mic, headset] = cards;
        let eq = l.eq.then(|| {
            let r = (LEFT, y, RIGHT, y + EQ_H);
            y += EQ_H + GAP;
            r
        });
        let footer = (LEFT, y - 2.0, RIGHT, y + 34.0);
        Frame { balance, mic, headset, eq, footer, height: footer.3 + 12.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> Caps {
        Caps::NONE
    }

    #[test]
    fn alpha_s_shows_everything_but_battery() {
        let l = Layout::for_caps(&MODELS[0].caps);
        assert!(l.balance && l.balance_slider && l.box_levels);
        assert!(l.mic && l.mic_status && l.sidetone && l.surround);
        assert!(l.eq && l.eq_needs_surround);
        assert!(!l.battery);
    }

    #[test]
    fn cards_follow_capabilities() {
        let l = Layout::for_caps(&caps());
        assert!(!l.balance && !l.mic && !l.eq && !l.battery);

        let l = Layout::for_caps(&Caps { sidetone: true, ..caps() });
        assert!(l.mic && l.sidetone && !l.surround && !l.mic_status);
        assert!(!l.balance && !l.eq);

        let l = Layout::for_caps(&Caps { box_levels: true, ..caps() });
        assert!(l.balance && !l.balance_slider);

        let l = Layout::for_caps(&Caps { eq_bands: 5, battery: true, ..caps() });
        assert!(l.eq && !l.eq_needs_surround && l.battery && !l.mic);
    }

    #[test]
    fn alpha_s_frame_matches_the_original_window() {
        let f = Frame::new(&Layout::for_caps(&MODELS[0].caps));
        assert_eq!(f.balance, Some((20.0, 84.0, 372.0, 300.0)));
        assert_eq!(f.mic, Some((388.0, 84.0, 740.0, 300.0)));
        assert_eq!(f.eq, Some((20.0, 316.0, 740.0, 662.0)));
        assert_eq!(f.footer, (20.0, 676.0, 740.0, 712.0));
        assert_eq!(f.height, 724.0);
    }

    #[test]
    fn headset_card_shares_a_row() {
        // Wireless: mic and headset side by side, no balance or EQ
        let wireless = Caps { sidetone: true, mic_mute_status: true, sidetone_level_max: 100, voice_prompts: true, ..caps() };
        let l = Layout::for_caps(&wireless);
        assert!(l.headset() && l.sidetone_level && l.voice_prompts && !l.auto_off);
        // Without auto power-off, voice prompts move up to the second row
        assert_eq!(l.headset_rows(), [Some(56.0), None, Some(102.0)]);
        let f = Frame::new(&l);
        // Row at the mic card's height (indicator + one row: 170), which is the tallest
        assert_eq!(f.mic, Some((20.0, 84.0, 372.0, 254.0)));
        assert_eq!(f.headset, Some((388.0, 84.0, 740.0, 254.0)));
        assert_eq!((f.balance, f.eq), (None, None));
        // All three: balance and mic on top, headset below at full width, and the EQ
        let all = Frame::new(&Layout::for_caps(&Caps { auto_off: &[0, 10], eq_bands: 10, box_levels: true, ..wireless }));
        assert_eq!(all.balance, Some((20.0, 84.0, 372.0, 300.0)));
        // Three rows: 56 + 3 × 46 + 4
        assert_eq!(all.headset, Some((20.0, 316.0, 740.0, 514.0)));
        assert_eq!(all.eq.map(|r| r.1), Some(530.0));
        // The Alpha S doesn't change: no headset card
        assert_eq!(Frame::new(&Layout::for_caps(&MODELS[0].caps)).headset, None);
    }

    #[test]
    fn card_heights_follow_their_rows() {
        let h = |c: Caps| Layout::for_caps(&c).card_heights();
        // Only the mic indicator: the minimum
        assert_eq!(h(Caps { mic_mute_status: true, ..caps() })[1], 124.0);
        // Only 7.1, no indicator: one row, the minimum
        assert_eq!(h(Caps { surround: true, ..caps() })[1], 120.0);
        // Headset card alone, with two rows: 56 + 2 × 46 + 4
        let l = Layout::for_caps(&Caps { auto_off: &[0, 10], voice_prompts: true, ..caps() });
        assert_eq!(l.card_heights()[2], 152.0);
        assert_eq!(Frame::new(&l).headset, Some((20.0, 84.0, 740.0, 236.0)));
    }

    #[test]
    fn alpha_s_cards_keep_their_height() {
        // Balance, and mic indicator + two rows: both are 216, as always
        let f = Frame::new(&Layout::for_caps(&MODELS[0].caps));
        assert_eq!(f.balance, Some((20.0, 84.0, 372.0, 300.0)));
        assert_eq!(f.mic, Some((388.0, 84.0, 740.0, 300.0)));
    }

    #[test]
    fn headset_values() {
        assert_eq!(level_share(Some(64), 128), Some(0.5));
        assert_eq!(level_share(Some(200), 100), Some(1.0));
        assert_eq!(level_share(None, 100), None);
        assert_eq!(level_text(Some(60), 100, 0), "60 %");
        assert_eq!(level_text(None, 100, 0), "—");
        // -5..+5 on a 0 to 10 bar
        assert_eq!((level_text(Some(5), 10, 5), level_text(Some(8), 10, 5), level_text(Some(0), 10, 5)), ("0".into(), "+3".into(), "-5".into()));
        // Five options fit whole in half a card (316 DIPs); seven don't
        let (chips, short) = auto_off_chips(&[0, 10, 20, 30, 60], 316.0, "Never", "Off", "min");
        assert!(!short);
        assert_eq!(chips[0], ("NEVER".to_string(), 0));
        assert_eq!(chips[1], ("10 MIN".to_string(), 10));
        let (chips, short) = auto_off_chips(&[0, 5, 10, 15, 20, 30, 60], 316.0, "Never", "Off", "min");
        assert!(short);
        assert_eq!((chips[0].0.as_str(), chips[2].0.as_str()), ("OFF", "10"));
        // At full width all seven fit
        assert!(!auto_off_chips(&[0, 5, 10, 15, 20, 30, 60], 684.0, "Never", "Off", "min").1);
    }

    #[test]
    fn hidden_cards_take_no_space() {
        let only_mic = Frame::new(&Layout::for_caps(&Caps { sidetone: true, ..caps() }));
        // One row without indicator: the minimum, 120
        assert_eq!(only_mic.mic, Some((20.0, 84.0, 740.0, 204.0)));
        assert_eq!((only_mic.balance, only_mic.eq), (None, None));
        assert!(only_mic.height < 400.0);

        let only_eq = Frame::new(&Layout::for_caps(&Caps { eq_bands: 10, ..caps() }));
        assert_eq!(only_eq.eq, Some((20.0, 84.0, 740.0, 430.0)));
        assert!(only_eq.footer.1 > 430.0);
    }

    #[test]
    fn pill_states() {
        let l = Layout::for_caps(&Caps { battery: true, ..Caps::NONE });
        let s = |battery, charging, asleep| Some(Status { battery, charging, asleep, ..Status::default() });
        assert_eq!(pill(0x4110, None, &l), Pill::Connected { firmware: 0x4110, battery: None });
        assert_eq!(pill(1, s(Some(80), false, false), &l).text("ON", "CHG", "OFF"), "ON  ·  80 %");
        assert_eq!(pill(1, s(Some(80), true, false), &l).text("ON", "CHG", "OFF"), "CHG  ·  80 %");
        assert_eq!(pill(1, s(None, false, true), &l), Pill::HeadsetOff);
        let no_battery = Layout::for_caps(&Caps::NONE);
        assert_eq!(pill(0x4110, s(Some(80), true, false), &no_battery).text("ON", "CHG", "OFF"), "ON  ·  FW 4110");
    }

    #[test]
    fn before_connect_matches_known_models() {
        let l = Layout::before_connect();
        assert!(l.eq && l.mic && l.balance && !l.battery);
    }
}
