//! Desktop SMTC. Callbacks post messages; only the window/worker own playback state.
use crate::audio::Snapshot;
use windows::{
    core::{factory, Result, HSTRING},
    Foundation::{TimeSpan, TypedEventHandler, Uri},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::RandomAccessStreamReference,
    Win32::{
        Foundation::HWND,
        System::WinRT::{
            ISystemMediaTransportControlsInterop, RoInitialize, RoUninitialize,
            RO_INIT_SINGLETHREADED,
        },
    },
};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const MESSAGE: u32 = WM_APP + 10;
pub const PLAY: usize = 1;
pub const PAUSE: usize = 2;
pub const NEXT: usize = 3;
pub const PREVIOUS: usize = 4;
pub const STOP: usize = 5;
pub const SEEK: usize = 6;

struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_SINGLETHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

pub struct Controls {
    controls: SystemMediaTransportControls,
    buttons: Option<i64>,
    seek: Option<i64>,
    metadata: Option<(String, String, String, String, String)>,
    timeline: Option<(u64, u64, String)>,
    // Release WinRT objects before balancing initialization on the window thread.
    _apartment: Apartment,
}

fn post(hwnd: usize, action: usize, position: isize) -> Result<()> {
    if unsafe { PostMessageW(hwnd as _, MESSAGE, action, position) } == 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(())
}

pub fn status(state: &str) -> MediaPlaybackStatus {
    match state {
        "Playing" => MediaPlaybackStatus::Playing,
        "Paused" => MediaPlaybackStatus::Paused,
        "Loading" => MediaPlaybackStatus::Changing,
        _ => MediaPlaybackStatus::Stopped,
    }
}

fn timespan(ms: u64) -> TimeSpan {
    TimeSpan {
        Duration: ms.min(i64::MAX as u64 / 10_000) as i64 * 10_000,
    }
}

impl Controls {
    pub fn new(hwnd: windows_sys::Win32::Foundation::HWND) -> Result<Self> {
        let apartment = Apartment::new()?;
        let interop: ISystemMediaTransportControlsInterop =
            factory::<SystemMediaTransportControls, _>()?;
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd))? };
        controls.SetIsEnabled(false)?;
        let mut result = Self {
            controls,
            buttons: None,
            seek: None,
            metadata: None,
            timeline: None,
            _apartment: apartment,
        };
        let target = hwnd as usize;
        result.buttons =
            Some(result.controls.ButtonPressed(&TypedEventHandler::new(
                move |_,
                      args: windows::core::Ref<
                    SystemMediaTransportControlsButtonPressedEventArgs,
                >| {
                    let action = match args
                        .as_ref()
                        .ok_or_else(windows::core::Error::empty)?
                        .Button()?
                    {
                        SystemMediaTransportControlsButton::Play => PLAY,
                        SystemMediaTransportControlsButton::Pause => PAUSE,
                        SystemMediaTransportControlsButton::Next => NEXT,
                        SystemMediaTransportControlsButton::Previous => PREVIOUS,
                        SystemMediaTransportControlsButton::Stop => STOP,
                        _ => return Ok(()),
                    };
                    post(target, action, 0)
                },
            ))?);
        result.seek = Some(result.controls.PlaybackPositionChangeRequested(
            &TypedEventHandler::new(
                move |_, args: windows::core::Ref<PlaybackPositionChangeRequestedEventArgs>| {
                    let ticks = args
                        .as_ref()
                        .ok_or_else(windows::core::Error::empty)?
                        .RequestedPlaybackPosition()?
                        .Duration;
                    post(
                        target,
                        SEEK,
                        (ticks.max(0) / 10_000).min(isize::MAX as i64) as isize,
                    )
                },
            ),
        )?);
        Ok(result)
    }

    pub fn update(&mut self, snapshot: &Snapshot) -> Result<()> {
        let active = snapshot.connected && !snapshot.now.id.is_empty();
        let controls = &self.controls;
        controls.SetIsEnabled(active)?;
        controls.SetIsPlayEnabled(
            active && matches!(snapshot.state.as_str(), "Playing" | "Paused" | "Error"),
        )?;
        controls.SetIsPauseEnabled(active && snapshot.state == "Playing")?;
        controls.SetIsStopEnabled(active)?;
        controls.SetIsNextEnabled(active)?;
        controls.SetIsPreviousEnabled(active)?;
        controls.SetPlaybackStatus(if active {
            status(&snapshot.state)
        } else {
            MediaPlaybackStatus::Closed
        })?;

        let metadata = if active {
            (
                snapshot.now.id.clone(),
                snapshot.now.title.clone(),
                snapshot.now.subtitle.clone(),
                snapshot.now.album.clone(),
                snapshot.now.artwork.clone(),
            )
        } else {
            Default::default()
        };
        if self.metadata.as_ref() != Some(&metadata) {
            let updater = controls.DisplayUpdater()?;
            updater.ClearAll()?;
            if active {
                updater.SetType(MediaPlaybackType::Music)?;
                updater.SetAppMediaId(&HSTRING::from(&metadata.0))?;
                let music = updater.MusicProperties()?;
                music.SetTitle(&HSTRING::from(&metadata.1))?;
                music.SetArtist(&HSTRING::from(&metadata.2))?;
                music.SetAlbumTitle(&HSTRING::from(&metadata.3))?;
                // Windows loads remote artwork asynchronously; never fetch on the UI thread.
                if metadata.4.starts_with("https://") || metadata.4.starts_with("http://") {
                    if let Ok(uri) = Uri::CreateUri(&HSTRING::from(&metadata.4)) {
                        if let Ok(image) = RandomAccessStreamReference::CreateFromUri(&uri) {
                            let _ = updater.SetThumbnail(&image);
                        }
                    }
                }
            }
            updater.Update()?;
            self.metadata = Some(metadata);
            self.timeline = None;
        }
        let duration = if active { snapshot.duration } else { 0 };
        let position = snapshot.position.min(duration);
        let timeline_key = (position / 1000, duration, snapshot.state.clone());
        if self.timeline.as_ref() != Some(&timeline_key) {
            let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
            timeline.SetStartTime(timespan(0))?;
            timeline.SetMinSeekTime(timespan(0))?;
            timeline.SetEndTime(timespan(duration))?;
            timeline.SetMaxSeekTime(timespan(duration))?;
            timeline.SetPosition(timespan(position))?;
            controls.UpdateTimelineProperties(&timeline)?;
            self.timeline = Some(timeline_key);
        }
        Ok(())
    }
}

impl Drop for Controls {
    fn drop(&mut self) {
        if let Some(token) = self.buttons.take() {
            let _ = self.controls.RemoveButtonPressed(token);
        }
        if let Some(token) = self.seek.take() {
            let _ = self.controls.RemovePlaybackPositionChangeRequested(token);
        }
        let _ = self.controls.SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = self.controls.SetIsEnabled(false);
        if let Ok(updater) = self.controls.DisplayUpdater() {
            let _ = updater.ClearAll();
            let _ = updater.Update();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_states_and_timeline_units() {
        assert_eq!(status("Playing"), MediaPlaybackStatus::Playing);
        assert_eq!(status("Paused"), MediaPlaybackStatus::Paused);
        assert_eq!(status("Loading"), MediaPlaybackStatus::Changing);
        assert_eq!(status("Error"), MediaPlaybackStatus::Stopped);
        assert_eq!(timespan(1234).Duration, 12_340_000);
        assert!(timespan(u64::MAX).Duration > 0);
    }

    #[test]
    #[ignore = "publishes a temporary SMTC session; requires an interactive Windows desktop"]
    fn desktop_smtc_roundtrip() {
        use windows::core::w;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WS_OVERLAPPED,
        };
        struct Window(windows_sys::Win32::Foundation::HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let window = Window(unsafe {
            CreateWindowExW(
                0,
                w!("STATIC").as_ptr(),
                w!("DeezNats SMTC test").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        });
        assert!(!window.0.is_null());
        let mut media = Controls::new(window.0).unwrap();
        let mut snapshot = Snapshot {
            connected: true,
            state: "Playing".into(),
            position: 1234,
            duration: 120_000,
            now: crate::catalog::Entry {
                id: "smtc-test".into(),
                title: "Test track".into(),
                subtitle: "Test artist".into(),
                album: "Test album".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        media.update(&snapshot).unwrap();
        assert!(media.controls.IsEnabled().unwrap());
        assert_eq!(
            media.controls.PlaybackStatus().unwrap(),
            MediaPlaybackStatus::Playing
        );
        let music = media
            .controls
            .DisplayUpdater()
            .unwrap()
            .MusicProperties()
            .unwrap();
        assert_eq!(music.Title().unwrap(), "Test track");
        assert_eq!(music.Artist().unwrap(), "Test artist");
        assert_eq!(music.AlbumTitle().unwrap(), "Test album");
        snapshot.state = "Paused".into();
        snapshot.position = 45_000;
        media.update(&snapshot).unwrap();
        assert_eq!(
            media.controls.PlaybackStatus().unwrap(),
            MediaPlaybackStatus::Paused
        );
        assert!(media.controls.IsPlayEnabled().unwrap());
        assert!(!media.controls.IsPauseEnabled().unwrap());
        snapshot.connected = false;
        media.update(&snapshot).unwrap();
        assert!(!media.controls.IsEnabled().unwrap());
        assert_eq!(
            media.controls.PlaybackStatus().unwrap(),
            MediaPlaybackStatus::Closed
        );
        // ClearAll removes the media type too; querying MusicProperties must fail
        // rather than exposing the previous track after disconnect.
        assert!(media
            .controls
            .DisplayUpdater()
            .unwrap()
            .MusicProperties()
            .is_err());
    }
}
