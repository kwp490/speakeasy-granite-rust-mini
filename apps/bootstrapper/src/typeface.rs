//! The wizard's type, and the only `unsafe` in this workspace.
//!
//! # Why this module exists
//!
//! `winsafe` builds one `HFONT` per process from `SPI_GETNONCLIENTMETRICS`'s
//! `lfMenuFont` — Segoe UI 9pt on a default Windows 11 — and sets it on each
//! control as that control is created. Nothing in the crate can change it
//! afterwards, because that is `WM_SETFONT` through `SendMessage`, which
//! `winsafe` marks `unsafe`.
//!
//! So every control in the setup wizard and the uninstall page drew at the
//! size Windows uses for *menu bars*, on a measure of about 105 characters,
//! with the heading, the step counter, the key line and the body all identical.
//! Measured 2026-08-26 on this machine at 240 dpi: `lfHeight = -30`, a 41 px
//! cell, inside a 1550 px-wide client area. Nothing was mis-scaled — the type
//! was correctly sized and simply too small to read comfortably, which was the
//! owner's report the same day.
//!
//! # Why `unsafe`, and why only here
//!
//! The workspace sets `unsafe_code = "forbid"` and every other crate inherits
//! it. `apps/bootstrapper/Cargo.toml` declares its own `deny` instead, and the
//! two functions below are the only `#[allow]`s under it — see that file for
//! the argument. The alternatives were painting the prose ourselves, which
//! leaves buttons and check boxes at 9pt and removes the `Static` controls
//! `scripts/Measure-NativeWindow.ps1 -Fit` reads, and rebuilding the wizard
//! from a dialog resource template, which needs an `.rc` compile step. Owner
//! decision 2026-08-26 between those three.
//!
//! Both calls are also what makes *weight* available, which
//! `docs/UI-GUIDE.md` recorded as impossible for as long as this file did not
//! exist: the heading is semibold now rather than emphasised by colour alone.
//!
//! # Sizes are a ratio of the system font, never absolute points
//!
//! A reader who has raised Windows' own text size — Accessibility → Text size,
//! which is exactly the person this module is for — gets a larger `lfMenuFont`
//! from `SPI_GETNONCLIENTMETRICS`. Forcing "12pt" would *shrink* the wizard for
//! them. Scaling their own height keeps the wizard larger than their system
//! font whatever they have set it to.
//!
//! Everything else in the `LOGFONT` is inherited: the face, the charset, the
//! quality and the pitch. A hardcoded "Segoe UI" would be wrong on a machine
//! whose UI language ships a different face, and `lfCharSet` defaulting to
//! `ANSI_CHARSET` would then pick whatever the font mapper liked.

use std::mem::size_of;

use winsafe::{self as w, co, guard, msg};

/// Body text, as a fraction of the system UI font: a third larger.
///
/// Chosen against the measure rather than by taste. At 9pt the wizard's
/// 588-logical-pixel content width holds about 105 characters, which is half
/// again the length prose is comfortable at; a third larger brings it to about
/// 80. It applies to the step counter, the key line, the body, the findings,
/// the status line, both radio groups, both check boxes, the vocabulary box and
/// every button — everything the reader is asked to read.
const BODY: (i32, i32) = (4, 3);

/// The step heading, and the only other size on the page.
///
/// Two sizes, not four. The heading answers "where am I", so it is the one
/// thing that has to be readable before the reader has decided to read
/// anything, and it carries [`co::FW::SEMIBOLD`] as well — the hierarchy the
/// page had none of.
const HEADING: (i32, i32) = (5, 3);

/// The wizard's two fonts, alive for as long as the window that draws with them.
///
/// Held by the window rather than made global: `winsafe`'s own font is a
/// process-wide `static mut` and this is deliberately not a second one. The
/// guards delete the GDI objects on drop, so dropping this while a control
/// still references a font would leave that control drawing with a deleted
/// handle — which is why [`crate::wizard::Wizard`] and
/// [`crate::uninstall_page`] keep it in an `Rc` beside the controls.
pub struct Typeface {
    /// `None` when the font could not be created, which leaves the control at
    /// `winsafe`'s 9pt. Deliberately not fatal: the layout below is sized for
    /// the larger font, so the failure mode is text that is smaller than
    /// intended inside boxes that are bigger than it needs — legible, and never
    /// clipped. A wizard that refuses to draw because a font is unavailable
    /// would be a worse answer to "the text is hard to read".
    body: Option<guard::DeleteObjectGuard<w::HFONT>>,
    heading: Option<guard::DeleteObjectGuard<w::HFONT>>,
}

impl Typeface {
    /// Build both fonts from the system's own UI font.
    #[must_use]
    pub fn new() -> Self {
        let base = system_ui_font();
        Self {
            body: base.as_ref().and_then(|base| scaled(base, BODY, None)),
            heading: base
                .as_ref()
                .and_then(|base| scaled(base, HEADING, Some(co::FW::SEMIBOLD))),
        }
    }

    /// Draw this control's text in the body size.
    pub fn body(&self, hwnd: &w::HWND) {
        set_font(self.body.as_deref(), hwnd);
    }

    /// Draw this control's text in the heading size and weight.
    pub fn heading(&self, hwnd: &w::HWND) {
        set_font(self.heading.as_deref(), hwnd);
    }
}

/// The `LOGFONT` Windows says its own menus are drawn with.
///
/// `lfMenuFont`, the same field `winsafe` reads, so the wizard's type is a
/// scaled version of what its controls would have used rather than a different
/// font at a different size. Already scaled to the system DPI, because this
/// process is system-DPI-aware — see the manifest for why it is that and not
/// `PerMonitorV2`.
fn system_ui_font() -> Option<w::LOGFONT> {
    let mut metrics = w::NONCLIENTMETRICS::default();
    // SAFETY: `SystemParametersInfo` is `unsafe` because `pv_param`'s type has
    // to match `action`, and a mismatch is a buffer overrun. `SPI` value
    // `GETNONCLIENTMETRICS` takes a `NONCLIENTMETRICS`, which is what is passed,
    // and `ui_param` is that struct's own size — the pairing `winsafe` itself
    // makes in `gui::privs::ui_font`. `NONCLIENTMETRICS::default` sets `cbSize`
    // to the same figure.
    #[allow(unsafe_code)]
    let read = unsafe {
        w::SystemParametersInfo(
            co::SPI::GETNONCLIENTMETRICS,
            u32::try_from(size_of::<w::NONCLIENTMETRICS>()).unwrap_or(0),
            &mut metrics,
            co::SPIF::NoValue,
        )
    };
    read.ok().map(|()| metrics.lfMenuFont.clone())
}

/// The same font, taller, and optionally heavier.
///
/// `lfHeight` is negative for a character height rather than a cell height, so
/// the multiplication carries the sign and the rounding stays away from zero.
fn scaled(
    base: &w::LOGFONT,
    (numerator, denominator): (i32, i32),
    weight: Option<co::FW>,
) -> Option<guard::DeleteObjectGuard<w::HFONT>> {
    let mut font = base.clone();
    font.lfHeight = base.lfHeight * numerator / denominator;
    if let Some(weight) = weight {
        font.lfWeight = weight;
    }
    w::HFONT::CreateFontIndirect(&font).ok()
}

/// `WM_SETFONT`, the one thing `winsafe` will not do for us.
///
/// A no-op for a font that could not be created, and for a control whose window
/// does not exist yet — this is called from the parent's `WM_CREATE`, after
/// `winsafe`'s own `before_on` handlers have created every child, which is the
/// earliest moment any of these handles is real.
fn set_font(font: Option<&w::HFONT>, hwnd: &w::HWND) {
    let Some(font) = font else {
        return;
    };
    // SAFETY: two `unsafe` calls, both narrow.
    //
    // `raw_copy` hands `SendMessage` a non-owning copy of the handle. Sound
    // because `Typeface` owns the only guard and outlives every control it is
    // applied to — the window holds it — so the font cannot be deleted while a
    // control still draws with it. That is the invariant `Typeface`'s own doc
    // states; breaking it is what this `unsafe` is guarding.
    //
    // `SendMessage` is `unsafe` for the general case of a message whose
    // `wparam`/`lparam` are pointers the receiver will dereference. `WM_SETFONT`
    // takes an `HFONT` in `wparam` and a redraw flag in `lparam`, both of which
    // `winsafe`'s `wm::SetFont` types, and it is documented as the supported way
    // to change a control's font. It is sent to a window this process created
    // and still owns.
    #[allow(unsafe_code)]
    unsafe {
        hwnd.SendMessage(msg::wm::SetFont {
            hfont: font.raw_copy(),
            redraw: true,
        });
    }
}
