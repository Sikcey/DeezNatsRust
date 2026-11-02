//! Capability means an implemented control surface, not a feature of Spotify's official app.
use serde::Serialize;
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Capabilities {
    pub downloads: bool,
    pub radio: bool,
    pub connect_devices: bool,
    pub jam_participation: bool,
    pub jam_management: bool,
    pub host_remote_control: bool,
    pub host_audio_stream: bool,
    pub equalizer: bool,
    pub output_selection: bool,
    pub normalization: bool,
    pub crossfade: bool,
}
pub(crate) const CAPABILITIES: Capabilities = Capabilities {
    // Soloist's volatile playback cache does not provide offline downloads.
    downloads: false,
    radio: true,
    connect_devices: true,
    jam_participation: true,
    jam_management: true,
    host_remote_control: true,
    host_audio_stream: true,
    // Soloist renders directly through WASAPI. Windows Volume mixer can route
    // its output, but that is not an implemented in-app output/DSP control.
    equalizer: false,
    output_selection: false,
    normalization: false,
    crossfade: false,
};
