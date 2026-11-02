//! Background threads. Each one opens its own HID handle, because on a synchronous handle a
//! pending ReadFile (waiting for a button) would block commands.
//!  - worker: runs commands, reads the state and applies settings on connect
//!  - listener: waits for buttons from the headset or its control box
//! Each event is queued and the UI is notified with `notify` (PostMessage on Windows,
//! invoke_from_event_loop on Linux), so the UI never blocks.

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::devices::{self, Band, ButtonEvent, Headset, Model, Status};
use crate::hid::HidError;
use crate::i18n::t;

/// Wakes the UI so it reads pending events.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

pub enum Cmd {
    Surround(bool),
    Sidetone(bool),
    SidetoneLevel(u8),
    AutoOff(u8),
    VoicePrompts(bool),
    Eq(Vec<Band>),
    Bands,
    Refresh,
}

pub enum Event {
    Connected { model: &'static Model, firmware: u16 },
    Disconnected,
    Hung,
    Status(Status),
    Button(ButtonEvent),
    Log(String),
}

/// What is applied every time the headset connects (the control box forgets it when unplugged).
/// Restores the last 7.1 and mic monitoring state (changed in the app or on the box), and the
/// headset card values. Comes from settings: `Settings::on_connect`.
#[derive(Clone)]
pub struct OnConnect {
    pub eq: Vec<Band>,
    pub surround: Option<bool>,
    pub sidetone: Option<bool>,
    pub sidetone_level: Option<u8>,
    pub auto_off: Option<u8>,
    pub voice_prompts: Option<bool>,
    /// Model the headset card values belong to (each one has its own scale).
    pub model: Option<String>,
}

pub struct Backend {
    cmds: Sender<Cmd>,
    pending_bands: Arc<Mutex<BTreeMap<usize, Band>>>,
    pub on_connect: Arc<Mutex<OnConnect>>,
}

#[derive(Clone)]
struct Ctx {
    notify: Notify,
    events: Sender<Event>,
    cmds: Sender<Cmd>,
}

impl Ctx {
    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
        (self.notify)();
    }
}

impl Backend {
    pub fn start(notify: Notify, on_connect: OnConnect) -> (Backend, Receiver<Event>) {
        let (cmd_tx, cmd_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        let backend = Backend {
            cmds: cmd_tx.clone(),
            pending_bands: Arc::default(),
            on_connect: Arc::new(Mutex::new(on_connect)),
        };
        let ctx = Ctx { notify, events: ev_tx, cmds: cmd_tx };
        let (pending, prefs) = (backend.pending_bands.clone(), backend.on_connect.clone());
        thread::spawn(move || worker(ctx, cmd_rx, pending, prefs));
        (backend, ev_rx)
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }

    /// Queues a band; when dragging fast only the last position of each one is sent.
    pub fn band(&self, index: usize, band: Band) {
        let first = {
            let mut p = self.pending_bands.lock().unwrap();
            let first = p.is_empty();
            p.insert(index, band);
            first
        };
        if first {
            self.send(Cmd::Bands);
        }
    }
}

fn apply_on_connect(dev: &dyn Headset, ctx: &Ctx, prefs: &OnConnect) -> Result<(), HidError> {
    let caps = dev.model().caps;
    if let Some(on) = prefs.sidetone.filter(|_| caps.sidetone) {
        if dev.set_sidetone(on)? && on {
            ctx.emit(Event::Log(t().mic_monitoring_on.into()));
        }
    }
    if let Some(on) = prefs.surround.filter(|_| caps.surround) {
        dev.set_surround(on)?;
    }
    // Headset card values, only if they are for this model: another model's volume uses another scale
    if prefs.model.as_deref() == Some(dev.model().name) {
        if let Some(level) = prefs.sidetone_level.filter(|_| caps.sidetone_level_max > 0) {
            dev.set_sidetone_level(level.min(caps.sidetone_level_max))?;
        }
        if let Some(minutes) = prefs.auto_off.filter(|m| caps.auto_off.contains(m)) {
            dev.set_auto_off(minutes)?;
        }
        if let Some(on) = prefs.voice_prompts.filter(|_| caps.voice_prompts) {
            dev.set_voice_prompts(on)?;
        }
    }
    if caps.eq_bands > 0 {
        let ok = dev.eq(&prefs.eq)?;
        ctx.emit(Event::Log(if ok { t().saved_eq_applied } else { t().eq_not_confirmed }.into()));
    }
    Ok(())
}

fn worker(ctx: Ctx, cmds: Receiver<Cmd>, pending: Arc<Mutex<BTreeMap<usize, Band>>>, prefs: Arc<Mutex<OnConnect>>) {
    let mut dev: Option<Box<dyn Headset>> = None;
    // Wireless headset off in the last state (the receiver stays open)
    let mut asleep = false;
    loop {
        let Some(d) = dev.as_ref() else {
            match devices::open() {
                Some(d) => {
                    let model = d.model();
                    ctx.emit(Event::Connected { model, firmware: d.firmware() });
                    let listener_ctx = ctx.clone();
                    thread::spawn(move || listener(listener_ctx, model));
                    let p = prefs.lock().unwrap().clone();
                    let ok = apply_on_connect(d.as_ref(), &ctx, &p).and_then(|_| d.status());
                    match ok {
                        Ok(s) => {
                            asleep = s.asleep;
                            ctx.emit(Event::Status(s));
                            dev = Some(d);
                        }
                        Err(_) => {
                            ctx.emit(Event::Hung);
                            thread::sleep(Duration::from_secs(2));
                        }
                    }
                }
                None => {
                    ctx.emit(Event::Disconnected);
                    thread::sleep(Duration::from_secs(2));
                }
            }
            continue;
        };

        let mut cmd = match cmds.recv_timeout(Duration::from_secs(5)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => Cmd::Refresh,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Coalesce consecutive refreshes (e.g. while holding a balance button)
        while matches!(cmd, Cmd::Refresh) {
            match cmds.try_recv() {
                Ok(next) => cmd = next,
                Err(_) => break,
            }
        }

        let result = (|| -> Result<bool, HidError> {
            match cmd {
                Cmd::Surround(on) => {
                    if d.set_surround(on)? && on && d.model().caps.eq_needs_surround {
                        // The EQ lives in the 7.1 DSP: resend it when 7.1 is turned on
                        let eq = prefs.lock().unwrap().eq.clone();
                        d.eq(&eq)?;
                    }
                }
                Cmd::Sidetone(on) => {
                    d.set_sidetone(on)?;
                }
                Cmd::SidetoneLevel(level) => d.set_sidetone_level(level)?,
                Cmd::AutoOff(minutes) => d.set_auto_off(minutes)?,
                Cmd::VoicePrompts(on) => d.set_voice_prompts(on)?,
                Cmd::Eq(bands) => {
                    let ok = d.eq(&bands)?;
                    ctx.emit(Event::Log(if ok { t().eq_applied } else { t().eq_not_confirmed }.into()));
                    return Ok(false);
                }
                Cmd::Bands => {
                    let bands = std::mem::take(&mut *pending.lock().unwrap());
                    let mut ok = true;
                    for (i, b) in bands {
                        ok &= d.eq_band(i, b)?;
                    }
                    if !ok {
                        ctx.emit(Event::Log(t().band_not_confirmed.into()));
                    }
                    return Ok(false);
                }
                Cmd::Refresh => {}
            }
            Ok(true)
        })();

        match result.and_then(|refresh| if refresh { d.status().map(Some) } else { Ok(None) }) {
            Ok(Some(s)) => {
                // On power-on the headset reverts to factory values: reapply the user's
                let woke = asleep && !s.asleep;
                asleep = s.asleep;
                let s = if woke {
                    let p = prefs.lock().unwrap().clone();
                    apply_on_connect(d.as_ref(), &ctx, &p).and_then(|_| d.status()).unwrap_or(s)
                } else {
                    s
                };
                ctx.emit(Event::Status(s));
            }
            Ok(None) => {}
            Err(e) => {
                // Unplugged or box hung: told apart when retrying to open it
                ctx.emit(Event::Log(format!("{}: {}", t().error, e.0)));
                let model = d.model();
                dev = None;
                if devices::open_model(model).is_some() {
                    ctx.emit(Event::Hung);
                }
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn listener(ctx: Ctx, model: &'static Model) {
    let Some(dev) = devices::open_model(model) else { return };
    while let Some(ev) = dev.wait_button() {
        ctx.emit(Event::Button(ev));
        let _ = ctx.cmds.send(Cmd::Refresh);
    }
}
