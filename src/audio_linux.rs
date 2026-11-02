//! Volume of a headset's "Game" and "Chat" outputs on Linux, via `pactl` (PipeWire or
//! PulseAudio). Changes are applied on a separate thread, keeping only the latest, so that
//! dragging the slider doesn't spawn a process per movement.

use std::process::Command;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

struct Sink {
    name: String,
    description: String,
    volume: f32,
}

/// `pactl list sinks` with LC_ALL=C (untranslated labels).
fn list_sinks() -> Vec<Sink> {
    let Ok(out) = Command::new("pactl").env("LC_ALL", "C").args(["list", "sinks"]).output() else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut sinks: Vec<Sink> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Sink #") {
            sinks.push(Sink { name: String::new(), description: String::new(), volume: 0.0 });
        } else if let Some(sink) = sinks.last_mut() {
            if let Some(v) = line.strip_prefix("Name: ") {
                sink.name = v.to_string();
            } else if let Some(v) = line.strip_prefix("Description: ") {
                sink.description = v.to_string();
            } else if let Some(v) = line.strip_prefix("Volume: ") {
                // "front-left: 42597 /  65% / -11.23 dB, ..."
                if let Some(pct) = v.split('/').nth(1).and_then(|p| p.trim().trim_end_matches('%').parse::<f32>().ok()) {
                    sink.volume = pct / 100.0;
                }
            }
        }
    }
    sinks
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
        let ours: Vec<Sink> = list_sinks()
            .into_iter()
            .filter(|s| s.description.contains(model) || s.name.contains(&sink_name))
            .collect();
        let by = |tag: &str| ours.iter().position(|s| s.description.contains(tag) || s.name.contains(tag));
        // If names don't tell them apart, the first output is Game and the second Chat (USB order)
        let (game, chat) = match (by("Game"), by("Chat")) {
            (Some(g), Some(c)) => (g, c),
            _ if ours.len() >= 2 => (0, 1),
            _ => return None,
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
                    let _ = Command::new("pactl").args(["set-sink-volume", &sink, &pct]).status();
                }
            }
        });
        Some(GameChat { names, tx })
    }

    pub fn get(&self) -> (f32, f32) {
        let (game, chat) = self.names.lock().unwrap().clone();
        let sinks = list_sinks();
        let vol = |name: &str| sinks.iter().find(|s| s.name == name).map_or(0.0, |s| s.volume);
        (vol(&game), vol(&chat))
    }

    pub fn set(&self, game: f32, chat: f32) {
        let _ = self.tx.send((game, chat));
    }
}
