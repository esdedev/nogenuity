//! Volume of a headset's "Game" and "Chat" outputs on Windows (Core Audio).
//!
//! The Alpha S has no command to set the game/chat balance; software balance
//! lowers the Windows volume of the opposite side, like the control box does.

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;

pub struct GameChat {
    game: IAudioEndpointVolume,
    chat: IAudioEndpointVolume,
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
            Some(GameChat { game: game?, chat: chat? })
        }
    }

    pub fn get(&self) -> (f32, f32) {
        unsafe {
            (
                self.game.GetMasterVolumeLevelScalar().unwrap_or(0.0),
                self.chat.GetMasterVolumeLevelScalar().unwrap_or(0.0),
            )
        }
    }

    pub fn set(&self, game: f32, chat: f32) {
        unsafe {
            let _ = self.game.SetMasterVolumeLevelScalar(game.clamp(0.0, 1.0), std::ptr::null());
            let _ = self.chat.SetMasterVolumeLevelScalar(chat.clamp(0.0, 1.0), std::ptr::null());
        }
    }
}
