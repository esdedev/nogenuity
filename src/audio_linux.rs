//! Volume of a headset's "Game" and "Chat" outputs on Linux, via `pactl` (PipeWire or
//! PulseAudio). Changes are applied on a separate thread, keeping only the latest, so that
//! dragging the slider doesn't spawn a process per movement.

use std::process::Command;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use crate::hid::log;

struct Sink {
    name: String,
    description: String,
    /// `None` if the output doesn't state a volume this code can read.
    volume: Option<f32>,
}

/// Writes a problem to the log, unless it's the one written last: the callers retry on every
/// status update. Keep sink names out of it, since they can hold a serial number and the log goes
/// into device reports.
fn log_problem(problem: &str) {
    static LAST: Mutex<String> = Mutex::new(String::new());
    let mut last = LAST.lock().unwrap();
    if *last != problem {
        log(problem);
        *last = problem.to_string();
    }
}

/// Runs `pactl` with LC_ALL=C (untranslated labels) and returns its output, or what went wrong.
fn pactl(args: &[&str]) -> Result<String, String> {
    let command = args[0];
    let out = Command::new("pactl").env("LC_ALL", "C").args(args).output().map_err(|e| format!("pactl {command}: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("pactl {command}: {} ({})", stderr.lines().next().unwrap_or("").trim(), out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn list_sinks() -> Result<Vec<Sink>, String> {
    Ok(parse_sinks(&pactl(&["list", "sinks"])?))
}

/// The sinks in the output of `pactl list sinks`.
fn parse_sinks(text: &str) -> Vec<Sink> {
    let mut sinks: Vec<Sink> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Sink #") {
            sinks.push(Sink { name: String::new(), description: String::new(), volume: None });
        } else if let Some(sink) = sinks.last_mut() {
            if let Some(v) = line.strip_prefix("Name: ") {
                sink.name = v.to_string();
            } else if let Some(v) = line.strip_prefix("Description: ") {
                sink.description = v.to_string();
            } else if let Some(v) = line.strip_prefix("Volume: ") {
                // "front-left: 42597 /  65% / -11.23 dB, ..."
                sink.volume = v.split('/').nth(1).and_then(|p| p.trim().trim_end_matches('%').parse::<f32>().ok()).map(|pct| pct / 100.0);
            }
        }
    }
    sinks
}

/// The volumes of the two outputs, only if both can be read.
fn levels_of(sinks: &[Sink], game: &str, chat: &str) -> Option<(f32, f32)> {
    let volume = |name: &str| sinks.iter().find(|s| s.name == name)?.volume;
    Some((volume(game)?, volume(chat)?))
}

pub struct GameChat {
    names: Arc<Mutex<(String, String)>>,
    tx: Sender<(f32, f32)>,
}

impl GameChat {
    /// Finds the outputs of model `model` (e.g. "Cloud Alpha S"; the sink name uses
    /// underscores instead of spaces).
    pub fn find(model: &str) -> Option<GameChat> {
        let sink_name = model.replace(' ', "_");
        let sinks = list_sinks().map_err(|e| log_problem(&e)).ok()?;
        let total = sinks.len();
        let ours: Vec<Sink> = sinks
            .into_iter()
            .filter(|s| s.description.contains(model) || s.name.contains(&sink_name))
            .collect();
        let by = |tag: &str| ours.iter().position(|s| s.description.contains(tag) || s.name.contains(tag));
        // If names don't tell them apart, the first output is Game and the second Chat (USB order)
        let (game, chat) = match (by("Game"), by("Chat")) {
            (Some(g), Some(c)) => (g, c),
            _ if ours.len() >= 2 => (0, 1),
            _ => {
                log_problem(&format!("balance: {} of {total} outputs belong to {model}, 2 are needed", ours.len()));
                return None;
            }
        };
        let names = Arc::new(Mutex::new((ours[game].name.clone(), ours[chat].name.clone())));
        let (tx, rx) = channel::<(f32, f32)>();
        let worker_names = names.clone();
        std::thread::spawn(move || {
            while let Ok(mut v) = rx.recv() {
                while let Ok(next) = rx.try_recv() {
                    v = next;
                }
                let (game, chat) = worker_names.lock().unwrap().clone();
                for (sink, level) in [(game, v.0), (chat, v.1)] {
                    let pct = format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round() as u32);
                    if let Err(e) = pactl(&["set-sink-volume", &sink, &pct]) {
                        log_problem(&e);
                    }
                }
            }
        });
        Some(GameChat { names, tx })
    }

    /// The current volumes, or `None` if they can't be read. Not reading them says nothing about
    /// what they are, so it mustn't be taken for a volume of zero.
    pub fn get(&self) -> Option<(f32, f32)> {
        let (game, chat) = self.names.lock().unwrap().clone();
        let sinks = list_sinks().map_err(|e| log_problem(&e)).ok()?;
        let levels = levels_of(&sinks, &game, &chat);
        if levels.is_none() {
            log_problem("balance: the outputs are gone or don't state a volume");
        }
        levels
    }

    pub fn set(&self, game: f32, chat: f32) {
        let _ = self.tx.send((game, chat));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From pactl 16.1, with null sinks standing in for the headset's outputs.
    const LISTING: &str = "Sink #1
	State: SUSPENDED
	Name: game_sink
	Description: HyperX Cloud Alpha S Game
	Driver: module-null-sink.c
	Sample Specification: s16le 2ch 44100Hz
	Channel Map: front-left,front-right
	Owner Module: 22
	Mute: no
	Volume: front-left: 42598 /  65% / -11.23 dB,   front-right: 42598 /  65% / -11.23 dB
	        balance 0.00
	Base Volume: 65536 / 100% / 0.00 dB
	Monitor Source: game_sink.monitor
	Latency: 1992884 usec, configured 2000000 usec
	Flags: DECIBEL_VOLUME LATENCY SET_FORMATS
	Properties:
		device.description = \"HyperX Cloud Alpha S Game\"
		device.class = \"abstract\"

Sink #2
	State: SUSPENDED
	Name: chat_sink
	Description: HyperX Cloud Alpha S Chat
	Volume: mono: 65536 / 100% / 0.00 dB
	Base Volume: 65536 / 100% / 0.00 dB
";

    #[test]
    fn reads_each_sink() {
        let sinks = parse_sinks(LISTING);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].name, "game_sink");
        assert_eq!(sinks[0].description, "HyperX Cloud Alpha S Game");
        assert_eq!(sinks[0].volume, Some(0.65));
        assert_eq!(sinks[1].volume, Some(1.0));
    }

    #[test]
    fn levels_need_both_outputs() {
        let sinks = parse_sinks(LISTING);
        assert_eq!(levels_of(&sinks, "game_sink", "chat_sink"), Some((0.65, 1.0)));
        assert_eq!(levels_of(&sinks, "game_sink", "gone"), None);
        assert_eq!(levels_of(&[], "game_sink", "chat_sink"), None);
    }

    #[test]
    fn an_unreadable_volume_is_not_zero() {
        for line in ["Volume: (invalid)", "Volume: front-left: 42598 / n/a / -11.23 dB"] {
            let sinks = parse_sinks(&format!("Sink #1\nName: game_sink\n{line}\nSink #2\nName: chat_sink\nVolume: mono: 1 / 100% / 0.00 dB\n"));
            assert_eq!(sinks[0].volume, None, "{line}");
            assert_eq!(levels_of(&sinks, "game_sink", "chat_sink"), None, "{line}");
        }
        // Silence that the system does report is still a volume
        let sinks = parse_sinks("Sink #1\nName: game_sink\nVolume: mono: 0 /   0% / -inf dB\n");
        assert_eq!(sinks[0].volume, Some(0.0));
    }
}
