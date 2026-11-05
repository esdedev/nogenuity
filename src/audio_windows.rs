//! Volume of a headset's "Game" and "Chat" outputs on Windows (Core Audio).
//!
//! The Alpha S has no command to set the game/chat balance; software balance
//! lowers the Windows volume of the opposite side, like the control box does.

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::balance::log_problem;

pub struct GameChat {
    game: IAudioEndpointVolume,
    chat: IAudioEndpointVolume,
    write_failed: AtomicBool,
}

impl GameChat {
    /// Finds the outputs whose name contains `model` (e.g. "Cloud Alpha S"). Call from a
    /// thread with COM initialized.
    pub fn find(model: &str) -> Option<GameChat> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).ok()?;
            let (mut game, mut chat) = (None, None);
            for i in 0..devices.GetCount().ok()? {
                let Ok(device) = devices.Item(i) else { continue };
                let Ok(store) = device.OpenPropertyStore(STGM_READ) else { continue };
                let Ok(value) = store.GetValue(&PKEY_Device_FriendlyName) else { continue };
                let Ok(name) = PropVariantToStringAlloc(&value) else { continue };
                let name: String = name.to_string().unwrap_or_default();
                if !name.contains(model) {
                    continue;
                }
                let volume: Option<IAudioEndpointVolume> = device.Activate(CLSCTX_ALL, None).ok();
                if name.contains("Game") {
                    game = volume;
                } else if name.contains("Chat") {
                    chat = volume;
                }
            }
            Some(GameChat { game: game?, chat: chat?, write_failed: AtomicBool::new(false) })
        }
    }

    /// The current volumes, or `None` if they can't be read. Not reading them says nothing about
    /// what they are, so it mustn't be taken for a volume of zero.
    pub fn get(&self) -> Option<(f32, f32)> {
        let levels = unsafe { self.game.GetMasterVolumeLevelScalar().and_then(|game| Ok((game, self.chat.GetMasterVolumeLevelScalar()?))) };
        levels.map_err(|e| log_problem(&format!("balance: reading the volume: {e}"))).ok()
    }

    pub fn set(&self, game: f32, chat: f32) {
        let (game, chat) = unsafe {
            (
                self.game.SetMasterVolumeLevelScalar(game.clamp(0.0, 1.0), std::ptr::null()),
                self.chat.SetMasterVolumeLevelScalar(chat.clamp(0.0, 1.0), std::ptr::null()),
            )
        };
        if let Err(e) = game.and(chat) {
            log_problem(&format!("balance: setting the volume: {e}"));
            self.write_failed.store(true, Ordering::SeqCst);
        }
    }

    /// Always: a write here has reached the output by the time it returns.
    pub fn settled(&self) -> bool {
        true
    }

    /// Whether a write failed since the last time this was asked.
    pub fn take_write_failure(&self) -> bool {
        self.write_failed.swap(false, Ordering::SeqCst)
    }
}
