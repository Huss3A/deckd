//! Mute state of the default speaker / microphone (Windows Core Audio), for state-aware buttons and the `mic_mute` media key.

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eCapture, eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The default playback device (speakers / headphones).
    Output,
    /// The default recording device (microphone).
    Mic,
}

fn endpoint(kind: Kind) -> Result<IAudioEndpointVolume, String> {
    // SAFETY: plain COM calls; COM is initialised per calling thread (already-initialised is fine)
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("audio: {e}"))?;
        let flow = if kind == Kind::Mic { eCapture } else { eRender };
        let dev = en.GetDefaultAudioEndpoint(flow, eConsole).map_err(|e| format!("audio: no default {kind:?} device ({e})"))?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).map_err(|e| format!("audio: {e}"))
    }
}

/// Is the default device of this kind muted right now?
pub fn muted(kind: Kind) -> Result<bool, String> {
    // SAFETY: COM call on a valid interface
    unsafe { endpoint(kind)?.GetMute().map(|b| b.as_bool()).map_err(|e| format!("audio: {e}")) }
}

/// Flip the mute switch of the default device; returns the new state (true = muted).
pub fn toggle(kind: Kind) -> Result<bool, String> {
    // SAFETY: COM calls on a valid interface
    unsafe {
        let v = endpoint(kind)?;
        let now = !v.GetMute().map_err(|e| format!("audio: {e}"))?.as_bool();
        v.SetMute(now, std::ptr::null()).map_err(|e| format!("audio: {e}"))?;
        Ok(now)
    }
}
