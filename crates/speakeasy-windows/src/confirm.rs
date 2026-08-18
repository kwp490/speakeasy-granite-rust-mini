//! Native modal confirmation.
//!
//! Deliberately a Windows `MessageBox` rather than an in-page dialog. The
//! compact transcriber is a `WS_EX_NOACTIVATE` window, so a `WebView` modal
//! cannot reliably take or trap focus — the user could be asked a question by a
//! surface that never receives their keystrokes. A native box is a separate,
//! focusable, system-modal window and does not have that problem.
//!
//! Strings are passed in rather than built here: they come from the caller's
//! catalog, so no user-facing prose lives in this crate.

/// What the user chose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confirmation {
    /// Go ahead with the action that prompted the question.
    Proceed,
    /// Leave everything as it is.
    Cancel,
}

/// Asks a yes/no question in a native modal box, defaulting to Cancel.
///
/// The default matters: this is used on the path where saying yes discards
/// speech, and a stray Enter or Space must not be what throws a dictation away.
///
/// If the box cannot be shown at all, the answer is `Cancel` — failing closed
/// keeps the recording rather than discarding it on a UI error.
#[must_use]
pub fn confirm_destructive_action(title: &str, message: &str) -> Confirmation {
    #[cfg(windows)]
    {
        use winsafe::co::MB;
        use winsafe::prelude::*;

        let flags = MB::YESNO | MB::ICONWARNING | MB::DEFBUTTON2 | MB::SETFOREGROUND | MB::TOPMOST;
        match winsafe::HWND::NULL.MessageBox(message, title, flags) {
            Ok(winsafe::co::DLGID::YES) => Confirmation::Proceed,
            _ => Confirmation::Cancel,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (title, message);
        Confirmation::Cancel
    }
}
