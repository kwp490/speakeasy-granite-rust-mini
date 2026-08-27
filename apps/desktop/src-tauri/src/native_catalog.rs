//! Source-language catalog for native Windows surfaces outside the `WebView`.

pub const TRAY_QUIT: &str = "Quit";
pub const TRAY_TOOLTIP: &str = "SpeakEasy";
pub const TRAY_SETTINGS: &str = "Settings";
pub const SETTINGS_WINDOW_TITLE: &str = "SpeakEasy settings";
pub const QUIT_DURING_DICTATION_TITLE: &str = "Close SpeakEasy?";
pub const QUIT_DURING_DICTATION_MESSAGE: &str = concat!(
    "A dictation is still recording. Closing SpeakEasy now discards it, and ",
    "the audio cannot be recovered.\r\n\r\n",
    "Choose No to keep recording, then use Stop & transcribe to finish."
);
pub const HUD_DOCK_MENU_SETTINGS: &str = "Settings";
pub const HUD_DOCK_MENU_CLOSE: &str = "Close SpeakEasy";
