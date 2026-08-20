//! The wizard window.
//!
//! Native Win32 through `winsafe`, not a Tauri window: the thing that provisions
//! `WebView2` is the thing this replaces, and repair mode has to draw on a
//! machine where something is already broken.
//!
//! The window, the eight steps, the navigation between them, and every control
//! any of them needs. **All of it is created in [`Wizard::new`]** — `winsafe`
//! panics if a control is built after its parent window, so there is no such
//! thing as building a page's controls when the user reaches it, and
//! [`Wizard::show_questions`] decides visibility instead.
//!
//! Half the steps report and half ask. The two kinds share one band of the
//! window on purpose: no step does both, the compatibility report is the tallest
//! thing here, and giving the questions their own space below it would add 170
//! logical pixels of emptiness to every other page.
//!
//! Three steps were placeholders until 2026-08-19, saying so rather than showing
//! a plausible blank. That message survives for whatever step is added next.
//!
//! Runs in a process with no console, launched by `relaunch_detached`. That
//! matters more than it looks: a console window belonging to `SpeakEasy` becomes
//! the delivery target for a dictation, and the last step of this wizard runs a
//! real one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use winsafe::prelude::*;
use winsafe::{self as w, co, gui};

use crate::{catalog, download, install, payload, probe, seed, smoke};

/// Layout, in DPI-independent units.
///
/// Grouped rather than inlined because the numbers relate to each other: the
/// body's height is the window's height less the header and the button strip,
/// and a change to any one of them that skips the others produces a window that
/// looks fine at 100% and overlaps at 250%. `UI-GUIDE.md`'s standing instruction
/// is to measure the running window rather than trust the declaration, and that
/// applies here too.
mod layout {
    /// Grown from 460 to 500 when the download step gained its own bar, rather
    /// than taking the space out of `NOTICE_HEIGHT`. Measured first: the notice
    /// holds 10.4 lines at 240 dpi and the compatibility report already uses 8,
    /// so the 40 px would have come out of two lines of headroom that the
    /// remaining steps are going to need. 500 logical is 1250 physical at 250%,
    /// which still fits a 2400-pixel-tall display with room over.
    pub const WINDOW: (i32, i32) = (620, 500);
    pub const MARGIN: i32 = 16;
    pub const HEADING_TOP: i32 = 16;
    pub const HEADING_HEIGHT: i32 = 26;
    pub const POSITION_TOP: i32 = 44;
    pub const POSITION_HEIGHT: i32 = 18;
    /// The one line on the page worth reading first, coloured by its tone.
    ///
    /// Its own band above the body rather than the body's first sentence,
    /// because the point of it is to be readable without reading the body. Two
    /// lines of room: the longest of them wraps to two at this measure, and a
    /// key line that clips is worse than no key line at all.
    pub const KEY_TOP: i32 = 68;
    pub const KEY_HEIGHT: i32 = 40;
    pub const BODY_TOP: i32 = 110;
    /// The step's own explanation. Short on purpose — the space below it
    /// belongs to what the step actually found or is asking for. Shrunk from
    /// 110 when the key line took its own band; the copy it holds shrank
    /// further than that, which is what the 2026-08-20 rewrite was for.
    pub const BODY_HEIGHT: i32 = 78;
    pub const NOTICE_TOP: i32 = 194;
    /// Where a step's findings and controls go. Sized for the longest thing it
    /// currently has to hold: the compatibility report, which runs to about
    /// eleven lines on a machine with a graphics card whose engines disagree.
    pub const NOTICE_HEIGHT: i32 = 170;
    /// Where a step that asks a question puts its controls.
    ///
    /// The same band as [`NOTICE_TOP`], because a step either reports something
    /// or asks something and no step does both — the notice label is hidden
    /// while controls are showing. Overlapping them deliberately rather than
    /// finding fresh space is what keeps the window one size: the compatibility
    /// report is the tallest thing the wizard shows, and giving the questions
    /// their own band below it would add 170 px of emptiness to every other
    /// step.
    pub const CONTROL_TOP: i32 = NOTICE_TOP;
    /// One radio button or check box. Sized for the text, not the glyph: at
    /// 250% a 20 px row clips the descenders on this font.
    pub const CONTROL_ROW: i32 = 24;
    /// The vocabulary box. Four or five lines at the wizard's text size, which
    /// is enough for a comma-separated list to wrap and still be read back
    /// without pretending this is the place to type fifty words.
    pub const EDIT_HEIGHT: i32 = 100;
    /// What a question step says back — a shortcut already in use, or why an
    /// option cannot be chosen. Below the controls rather than above them, so
    /// the answer appears where the eye already is after choosing.
    pub const STATUS_TOP: i32 = 300;
    pub const STATUS_HEIGHT: i32 = 64;
    /// The download step's own bar.
    ///
    /// A second bar rather than reusing the step indicator below, which counts
    /// steps: one control cannot honestly mean "step 3 of 7" and "41% of 453 MB"
    /// at the same time, and a bar that changes meaning between steps is a bar
    /// nobody reads. Created with every other control and hidden except on the
    /// step that owns it — `winsafe` panics if a control is created after its
    /// parent window is, so nothing here can be built on demand.
    pub const TRANSFER_TOP: i32 = 376;
    pub const TRANSFER_HEIGHT: i32 = 14;
    pub const PROGRESS_TOP: i32 = 416;
    pub const PROGRESS_HEIGHT: i32 = 8;
    pub const BUTTON_TOP: i32 = 444;
    pub const BUTTON: (i32, i32) = (96, 28);
    pub const BUTTON_GAP: i32 = 8;
}

#[derive(Clone)]
pub struct Wizard {
    window: gui::WindowMain,
    heading: gui::Label,
    position: gui::Label,
    /// The step's key line. Coloured from [`catalog::Step::key_tone`].
    key: gui::Label,
    body: gui::Label,
    notice: gui::Label,
    /// What a question step says back. Shares the notice's band; see
    /// [`layout::STATUS_TOP`].
    status: gui::Label,
    /// Which configuration to install. Two buttons even though only one of them
    /// can be chosen today, because the choice is real and the reason the other
    /// is unavailable is worth showing — a machine with a graphics card and no
    /// mention of it reads as a machine setup did not look at.
    provider: gui::RadioGroup,
    /// The activation shortcut, from [`shortcut_choices`].
    shortcut: gui::RadioGroup,
    /// Words to protect, comma-separated.
    ///
    /// Comma-separated since 2026-08-20, and one per line before that. The line
    /// form was more typing and one more thing to remember, and a list is what
    /// people already know how to type — the box still accepts newlines so a
    /// user who types them anyway loses nothing.
    words: gui::Edit,
    /// Whether transcripts survive closing the app. Unchecked by default.
    keep_transcripts: gui::CheckBox,
    /// Whether the diagnostic log is written to disk. Checked by default,
    /// matching the app's own setting.
    disk_logging: gui::CheckBox,
    transfer: gui::ProgressBar,
    progress: gui::ProgressBar,
    back: gui::Button,
    next: gui::Button,
    cancel: gui::Button,
    /// Runs the engine check again. Only visible on the step that owns it.
    retry: gui::Button,
    /// Which step is showing.
    ///
    /// `Rc<Cell<_>>` because `winsafe` requires `'static` event closures, so the
    /// handlers own a clone of this struct rather than borrowing it.
    step: Rc<Cell<usize>>,
    /// What this machine can run, probed once.
    ///
    /// Once, not per visit to the first step: `free_vram_bytes` moves, and a
    /// report that changed each time the user pressed Back would look like the
    /// machine was changing under them. Re-probing belongs on an explicit
    /// action, not on navigation.
    machine: Rc<probe::MachineReport>,
    /// What setup may do to this machine, decided once alongside the probe.
    install: Rc<install::Decision>,
    /// The transfer, once the download step has started one.
    ///
    /// `RefCell` rather than `Cell` because a `Run` is not `Copy` and the timer
    /// has to read it without taking it. `None` until the user reaches the step:
    /// nothing downloads because a window opened.
    run: Rc<RefCell<Option<download::Run>>>,
    /// The engine check, once the last step has started one.
    ///
    /// Same shape as `run` and for the same reasons: not `Copy`, read by the
    /// timer without taking it, and `None` until the user arrives -- a wizard
    /// that loaded a 2 GB model because a window opened would be worse than one
    /// that waits to be asked.
    verify: Rc<RefCell<Option<smoke::Run>>>,
    /// Set once the engine check reported finished, so its verdict is written
    /// once rather than on every timer tick.
    verify_settled: Rc<Cell<bool>>,
    /// Set once the run reported finished, so the completion message is written
    /// once rather than every timer tick.
    settled: Rc<Cell<bool>>,
    /// Whether everything the download step is responsible for is on disk and
    /// verified.
    ///
    /// Separate from [`Self::settled`], which only means the run stopped — a
    /// failed run is settled and not ready. Kept as its own flag because it is
    /// what gates Next, and `show_step` re-applies that gate on every visit,
    /// including visits from Back after the transfer already succeeded.
    ready: Rc<Cell<bool>>,
    /// What colour each label that can change tone is currently showing.
    ///
    /// Cells rather than an argument to a paint routine, because the colour is
    /// decided where the text is written and applied much later, in the
    /// `WM_CTLCOLORSTATIC` the control sends while painting itself. Nothing else
    /// gets to choose: a label whose text and tone were set from two places
    /// would eventually show a green failure.
    key_tone: Rc<Cell<catalog::Tone>>,
    notice_tone: Rc<Cell<catalog::Tone>>,
    status_tone: Rc<Cell<catalog::Tone>>,
    /// Whether the shortcut showing on the shortcut step is one Windows will
    /// actually give the app.
    ///
    /// Checked by registering it, which is the only answer that is not a guess:
    /// a combination another program already owns cannot be told from a free
    /// one by looking at it. Gates Next, because a shortcut that does not work
    /// is discovered by the user pressing it and nothing happening.
    shortcut_free: Rc<Cell<bool>>,
}

/// Indices into [`catalog::STEPS`] for the steps that have content.
///
/// Named rather than spelled as numbers at the point of use: the step order is
/// still settling, and a bare `3` in `show_step` would silently start describing
/// the wrong step the first time one is inserted above it.
const STEP_COMPATIBILITY: usize = 0;
const STEP_PROVIDER: usize = 1;
const STEP_DOWNLOAD: usize = 2;
const STEP_INSTALL: usize = 3;
const STEP_SHORTCUT: usize = 4;
const STEP_WORDS: usize = 5;
/// Retention and diagnostic logging. Its own step rather than a corner of
/// another one: both answers are about what `SpeakEasy Mini` keeps, and the
/// retention default is a privacy promise the user should meet on its own page
/// rather than beneath a list of words.
const STEP_PRIVACY: usize = 6;
/// The engine check, and the last step. `STEPS.len() - 1` rather than a
/// literal so adding a step ahead of it does not silently point this at copy
/// about something else.
const STEP_VERIFY: usize = catalog::STEPS.len() - 1;

/// The controls belonging to the steps that ask something.
///
/// A struct rather than a tuple returned from [`Wizard::question_controls`]:
/// six same-shaped handles in a row is exactly the kind of list two of which
/// get swapped without the compiler minding.
struct Questions {
    provider: gui::RadioGroup,
    shortcut: gui::RadioGroup,
    words: gui::Edit,
    keep_transcripts: gui::CheckBox,
    disk_logging: gui::CheckBox,
    status: gui::Label,
}

/// One offered activation shortcut.
///
/// The binding string and the key it decomposes into are held together because
/// they are two spellings of one fact, and the one thing that must never happen
/// is setup verifying one combination and recording another — the user would
/// then be told their shortcut is free and find that a different one was
/// installed.
struct ShortcutChoice {
    /// As `speakeasy_storage::Settings` spells it, which is what the seed file
    /// carries and what `tauri_plugin_global_shortcut` parses.
    binding: &'static str,
    label: &'static str,
    modifiers: co::MOD,
    key: co::VK,
}

/// The shortcuts setup offers, best first.
///
/// A short list rather than a key-capture control. Capturing an arbitrary
/// combination means reproducing the app's own parser, its reserved-key rules
/// and its conflict reporting inside an installer that runs once — and the
/// app's Settings page already does all of that, for a user who has by then
/// seen the app work. What setup owes is a working default and a way out of a
/// collision, which is three named alternatives.
///
/// `Ctrl+Alt+P` leads because it is the product's own default, and it is
/// deliberately not `SpeakEasy`'s `Ctrl+Alt+L`: the two install side by side
/// and must never fight over a shortcut.
///
/// A function rather than a `const`: `co::MOD`'s `BitOr` is an ordinary trait
/// implementation, so the combinations below cannot be built in a constant.
fn shortcut_choices() -> [ShortcutChoice; 3] {
    [
        ShortcutChoice {
            binding: "Ctrl+Alt+P",
            label: catalog::SHORTCUT_CTRL_ALT_P,
            modifiers: co::MOD::CONTROL | co::MOD::ALT,
            key: co::VK::CHAR_P,
        },
        ShortcutChoice {
            binding: "Ctrl+Alt+D",
            label: catalog::SHORTCUT_CTRL_ALT_D,
            modifiers: co::MOD::CONTROL | co::MOD::ALT,
            key: co::VK::CHAR_D,
        },
        ShortcutChoice {
            binding: "Ctrl+Shift+Space",
            label: catalog::SHORTCUT_CTRL_SHIFT_SPACE,
            modifiers: co::MOD::CONTROL | co::MOD::SHIFT,
            key: co::VK::SPACE,
        },
    ]
}

/// What a [`catalog::Tone`] paints with.
///
/// Colour and nothing else. **Bold is not available here**: emphasising a
/// label's font means `WM_SETFONT`, `winsafe` only sends messages through an
/// `unsafe` call, and this workspace sets `unsafe_code = "forbid"`. So the
/// emphasis a reader gets is a colour plus the fact that the key line is one
/// short line on its own — and every tone is also carried by the words, per
/// `UI-GUIDE.md`'s rule that colour is never the only signal.
///
/// Fixed values rather than system colours for the three that mean something.
/// The window is drawn on the dialog face (`COLOR::BTNFACE`), which these are
/// chosen to sit on; [`catalog::Tone::Plain`] defers to the system so ordinary
/// text still follows the user's theme.
fn tone_color(tone: catalog::Tone) -> w::COLORREF {
    match tone {
        catalog::Tone::Plain => w::GetSysColor(co::COLOR::WINDOWTEXT),
        catalog::Tone::Accent => w::COLORREF::from_rgb(0x0a, 0x3d, 0x91),
        catalog::Tone::Warning => w::COLORREF::from_rgb(0x9b, 0x1c, 0x1c),
        catalog::Tone::Good => w::COLORREF::from_rgb(0x0b, 0x5a, 0x1e),
    }
}

/// Write a label and record the colour it must paint in.
///
/// One function for both halves, because they cannot be allowed to disagree: a
/// static control paints its own text, so the colour is applied much later, in
/// the `WM_CTLCOLORSTATIC` it sends on the way. Setting the text from one place
/// and the tone from another is how a failure ends up green.
///
/// The explicit invalidate is for the case where only the tone changed —
/// `SetWindowText` dirties the control by itself, but re-tinting the same words
/// would otherwise wait for something else to dirty the rectangle.
fn say(label: &gui::Label, cell: &Cell<catalog::Tone>, text: &str, tone: catalog::Tone) {
    cell.set(tone);
    let _ = label.hwnd().SetWindowText(text);
    let _ = label.hwnd().InvalidateRect(None, true);
}

/// A hotkey id nothing else in this process uses.
///
/// Registered and immediately released, only to find out whether Windows will
/// hand it over at all.
const PROBE_HOTKEY_ID: i32 = 0x5350;

/// Whether a step asks the user something rather than reporting.
///
/// One predicate, because three places need the answer — what to show, what to
/// write into, and whether the notice label is in the way — and three copies of
/// the same list is how a step gets added to two of them.
const fn asks_a_question(index: usize) -> bool {
    matches!(
        index,
        STEP_PROVIDER | STEP_SHORTCUT | STEP_WORDS | STEP_PRIVACY
    )
}

/// How often the window reads the transfer's progress.
///
/// A timer rather than a callback from the worker: `download_to_file` reports
/// nothing until it returns, so progress is the size of the partial file on
/// disk, which only a poll can see. 250 ms is fast enough that the bar moves
/// visibly and slow enough that the stat costs nothing.
const POLL_ID: usize = 1;
const POLL_MS: u32 = 250;

/// Resolution of the transfer bar.
///
/// Per mille rather than per cent because a 2.3 GB download moves a hundred-step
/// bar once every twenty-three megabytes, which on a slow line looks stopped.
const TRANSFER_STEPS: u32 = 1000;

impl Wizard {
    pub fn new() -> Self {
        // Probed before any control exists, because the provider step's buttons
        // depend on the answer and `winsafe` panics if a control is created
        // after its parent window. The probe used to run further down, with the
        // rest of the state; moving it changes nothing about when the machine is
        // read, only about what can read it.
        let machine = probe::run();
        let window = gui::WindowMain::new(gui::WindowMainOpts {
            title: catalog::WINDOW_TITLE,
            size: gui::dpi(layout::WINDOW.0, layout::WINDOW.1),
            // No maximize and no resize box. The step content is prose at a
            // fixed measure; a resizable window here buys nothing and gives
            // every future step a layout problem to solve.
            style: gui::WindowMainOpts::default().style | co::WS::MINIMIZEBOX,
            ..Default::default()
        });

        let content_width = layout::WINDOW.0 - (layout::MARGIN * 2);
        let [heading, position, key, body, notice] = Self::page_labels(&window, content_width);
        let transfer = gui::ProgressBar::new(
            &window,
            gui::ProgressBarOpts {
                position: gui::dpi(layout::MARGIN, layout::TRANSFER_TOP),
                size: gui::dpi(content_width, layout::TRANSFER_HEIGHT),
                range: (0, TRANSFER_STEPS),
                value: 0,
                ..Default::default()
            },
        );
        let progress = gui::ProgressBar::new(
            &window,
            gui::ProgressBarOpts {
                position: gui::dpi(layout::MARGIN, layout::PROGRESS_TOP),
                size: gui::dpi(content_width, layout::PROGRESS_HEIGHT),
                range: (0, u32::try_from(catalog::STEPS.len()).unwrap_or(1)),
                value: 1,
                ..Default::default()
            },
        );

        let [cancel, next, back, retry] = Self::button_row(&window);
        let questions = Self::question_controls(&window, content_width, &machine);

        let wizard = Self {
            window,
            heading,
            position,
            key,
            body,
            notice,
            status: questions.status,
            provider: questions.provider,
            shortcut: questions.shortcut,
            words: questions.words,
            keep_transcripts: questions.keep_transcripts,
            disk_logging: questions.disk_logging,
            transfer,
            progress,
            back,
            next,
            cancel,
            retry,
            step: Rc::new(Cell::new(0)),
            key_tone: Rc::new(Cell::new(catalog::Tone::Plain)),
            notice_tone: Rc::new(Cell::new(catalog::Tone::Plain)),
            status_tone: Rc::new(Cell::new(catalog::Tone::Plain)),
            verify: Rc::new(RefCell::new(None)),
            verify_settled: Rc::new(Cell::new(false)),
            machine: Rc::new(machine),
            install: Rc::new(install::decide_now()),
            run: Rc::new(RefCell::new(None)),
            settled: Rc::new(Cell::new(false)),
            ready: Rc::new(Cell::new(false)),
            // Nothing is known until the shortcut step registers one. Starting
            // at `false` rather than `true` means a step that somehow never runs
            // its check blocks rather than waves the user through.
            shortcut_free: Rc::new(Cell::new(false)),
        };
        wizard.wire_events();
        wizard
    }

    /// The five stacked labels every step paints into, top to bottom.
    ///
    /// Extracted from `new` because it pushed that function past the
    /// hundred-line lint when the key line was added; the grouping is the right
    /// one anyway — these are one column of text at one measure, and their tops
    /// only mean anything relative to each other.
    fn page_labels(window: &gui::WindowMain, content_width: i32) -> [gui::Label; 5] {
        let band = |top: i32, height: i32, text: &str| {
            gui::Label::new(
                window,
                gui::LabelOpts {
                    text,
                    position: gui::dpi(layout::MARGIN, top),
                    size: gui::dpi(content_width, height),
                    ..Default::default()
                },
            )
        };
        [
            band(layout::HEADING_TOP, layout::HEADING_HEIGHT, ""),
            band(layout::POSITION_TOP, layout::POSITION_HEIGHT, ""),
            band(layout::KEY_TOP, layout::KEY_HEIGHT, ""),
            band(layout::BODY_TOP, layout::BODY_HEIGHT, ""),
            band(
                layout::NOTICE_TOP,
                layout::NOTICE_HEIGHT,
                catalog::STEP_NOT_BUILT,
            ),
        ]
    }

    /// Every control that belongs to a step which asks something.
    ///
    /// Built here, with everything else, because `winsafe` panics if a control
    /// is created after its parent window — so there is no such thing as
    /// building a page's controls when the user reaches it. They exist from the
    /// first paint and [`Self::show_questions`] decides which are visible.
    fn question_controls(
        window: &gui::WindowMain,
        content_width: i32,
        machine: &probe::MachineReport,
    ) -> Questions {
        let row = |index: i32| {
            gui::dpi(
                layout::MARGIN,
                layout::CONTROL_TOP + layout::CONTROL_ROW * index,
            )
        };
        let wide = gui::dpi(content_width, layout::CONTROL_ROW);
        // A graphics-card install needs a CUDA-built worker to exist, and
        // Granite's GPU support is compiled into the worker rather than loaded
        // beside it — so no manifest entry means no such install, whatever the
        // card can do. Offered only when both are true; the step says which half
        // is missing.
        //
        // `preferred_provider`, not `is_qualified`: qualification means an
        // execution test has passed on this card, and setup has not run one at
        // this point — `is_qualified` is false on every machine here, so
        // reading it would disable the option even once a worker exists.
        // `GpuQualification::preferred_provider`'s own doc names this as where
        // a GPU override belongs.
        let graphics_card = machine.admissibility.preferred_provider()
            == speakeasy_models::ExecutionProvider::Cuda
            && download::graphics_card_configuration_published();
        Questions {
            provider: gui::RadioGroup::new(
                window,
                &[
                    gui::RadioButtonOpts {
                        text: catalog::PROVIDER_GRAPHICS_CARD,
                        position: row(0),
                        size: wide,
                        selected: graphics_card,
                        ..Default::default()
                    },
                    gui::RadioButtonOpts {
                        text: catalog::PROVIDER_PROCESSOR,
                        position: row(1),
                        size: wide,
                        selected: !graphics_card,
                        ..Default::default()
                    },
                ],
            ),
            shortcut: gui::RadioGroup::new(
                window,
                &shortcut_choices()
                    .iter()
                    .enumerate()
                    .map(|(index, choice)| gui::RadioButtonOpts {
                        text: choice.label,
                        position: row(i32::try_from(index).unwrap_or(0)),
                        size: wide,
                        selected: index == 0,
                        ..Default::default()
                    })
                    .collect::<Vec<_>>(),
            ),
            words: gui::Edit::new(
                window,
                gui::EditOpts {
                    text: "",
                    position: gui::dpi(layout::MARGIN, layout::CONTROL_TOP),
                    width: gui::dpi_x(content_width),
                    height: gui::dpi_y(layout::EDIT_HEIGHT),
                    // `WANTRETURN` is what makes Enter add a line rather than
                    // press the default button, which on this window is Next —
                    // a user typing a second word would otherwise leave the step.
                    control_style: co::ES::MULTILINE
                        | co::ES::WANTRETURN
                        | co::ES::AUTOVSCROLL
                        | co::ES::NOHIDESEL,
                    ..Default::default()
                },
            ),
            keep_transcripts: gui::CheckBox::new(
                window,
                gui::CheckBoxOpts {
                    text: catalog::KEEP_TRANSCRIPTS,
                    position: row(0),
                    size: wide,
                    // Unchecked, and this is the owner's decision of 2026-08-19
                    // rather than a convenience: it matches what the app already
                    // does, and a privacy-preserving default needs no
                    // justification to the user.
                    check_state: co::BST::UNCHECKED,
                    ..Default::default()
                },
            ),
            disk_logging: gui::CheckBox::new(
                window,
                gui::CheckBoxOpts {
                    text: catalog::DISK_LOGGING,
                    position: row(2),
                    size: wide,
                    // Checked, matching `Settings`' own default. The log records
                    // error codes and counters, never transcript text.
                    check_state: co::BST::CHECKED,
                    ..Default::default()
                },
            ),
            status: gui::Label::new(
                window,
                gui::LabelOpts {
                    text: "",
                    position: gui::dpi(layout::MARGIN, layout::STATUS_TOP),
                    size: gui::dpi(content_width, layout::STATUS_HEIGHT),
                    ..Default::default()
                },
            ),
        }
    }

    fn button(parent: &gui::WindowMain, text: &str, left: i32) -> gui::Button {
        gui::Button::new(
            parent,
            gui::ButtonOpts {
                text,
                position: gui::dpi(left, layout::BUTTON_TOP),
                width: gui::dpi_x(layout::BUTTON.0),
                height: gui::dpi_y(layout::BUTTON.1),
                ..Default::default()
            },
        )
    }

    /// The button row, right-aligned in reading order from the right edge.
    ///
    /// Cancel, Next, Back is the Windows convention and the one the user's other
    /// installers follow. Retry sits *left* of Back rather than among them, so
    /// showing it on the engine-check step moves nothing the user is already
    /// aiming at.
    ///
    /// Extracted from `new` only because it pushed that function past the
    /// hundred-line lint; the grouping happens to be the right one anyway.
    fn button_row(window: &gui::WindowMain) -> [gui::Button; 4] {
        let right = layout::WINDOW.0 - layout::MARGIN;
        let slot =
            |index: i32| right - (layout::BUTTON.0 * index) - (layout::BUTTON_GAP * (index - 1));
        [
            Self::button(window, catalog::CANCEL, slot(1)),
            Self::button(window, catalog::NEXT, slot(2)),
            Self::button(window, catalog::BACK, slot(3)),
            Self::button(window, catalog::RETRY, slot(4)),
        ]
    }

    fn wire_events(&self) {
        let on_create = self.clone();
        self.window.on().wm_create(move |_| {
            on_create.show_step()?;
            // Runs for the window's whole life rather than being started and
            // stopped around the download step. A timer that only exists while
            // transferring is a timer that has to be killed on every path out of
            // that step — Back, Next, Cancel, and the window being destroyed —
            // and one missed path leaves a callback firing at a dead window.
            // Ticking 250 ms of nothing costs less than that.
            on_create.window.hwnd().SetTimer(POLL_ID, POLL_MS, None)?;
            Ok(0)
        });

        // The only chance to colour a static control's text: it paints itself,
        // and asks its parent for the ink on the way. Three labels can change
        // tone; every other static keeps the system colour, which is what
        // `Tone::Plain` resolves to.
        let on_color = self.clone();
        self.window.on().wm_ctl_color_static(move |p| {
            let tone = if p.hwnd == *on_color.key.hwnd() {
                on_color.key_tone.get()
            } else if p.hwnd == *on_color.notice.hwnd() {
                on_color.notice_tone.get()
            } else if p.hwnd == *on_color.status.hwnd() {
                on_color.status_tone.get()
            } else {
                catalog::Tone::Plain
            };
            let _ = p.hdc.SetTextColor(tone_color(tone));
            // Transparent text, and the returned brush still erases the
            // background — the standard pairing. Leaving the mode opaque paints
            // each run of text on the device context's own background colour,
            // which is white, so coloured lines arrive in white boxes.
            let _ = p.hdc.SetBkMode(co::BKMODE::TRANSPARENT);
            Ok(w::HBRUSH::GetSysColorBrush(co::COLOR::BTNFACE)?)
        });

        let on_tick = self.clone();
        self.window.on().wm_timer(POLL_ID, move || {
            on_tick.poll_transfer();
            on_tick.poll_verify();
            Ok(())
        });

        let on_retry = self.clone();
        self.retry.on().bn_clicked(move || {
            on_retry.begin_verify();
            Ok(())
        });

        // Choosing a different shortcut re-asks Windows about it. Without this
        // the step would report on whatever was selected when the user arrived,
        // which is the answer for a combination they have since changed their
        // mind about — and it would be shown right next to the one they picked.
        let on_shortcut = self.clone();
        self.shortcut.on().bn_clicked(move || {
            let (message, tone) = on_shortcut.verify_selected_shortcut();
            on_shortcut.set_status(&message, tone);
            Ok(())
        });

        // The provider step's own explanation depends on which button is
        // selected, so it is rewritten the same way.
        let on_provider = self.clone();
        self.provider.on().bn_clicked(move || {
            let (message, tone) = on_provider.describe_provider_choice();
            on_provider.set_status(&message, tone);
            Ok(())
        });

        // What the box will actually hand over, counted as it is typed.
        //
        // The count is the whole reason a comma-separated box is safe to ask
        // for: a reader who typed "Kenneth Perry, Anthropic" and meant two
        // words can see that setup read two, and one who forgot the comma can
        // see that it read one. It is [`seed::parse_vocabulary`]'s own answer
        // rather than a second count of the same text, so it cannot be right
        // about a list the seed file disagrees with.
        let on_words = self.clone();
        self.words.on().en_change(move || {
            on_words.show_word_count();
            Ok(())
        });

        let on_back = self.clone();
        self.back.on().bn_clicked(move || {
            let current = on_back.step.get();
            if current > 0 {
                on_back.step.set(current - 1);
                on_back.show_step()?;
            }
            Ok(())
        });

        let on_next = self.clone();
        self.next.on().bn_clicked(move || {
            let current = on_next.step.get();
            // Leaving the install step is what performs the install. Blocking
            // the message loop while it copies is a known limitation, not an
            // oversight: progress reporting arrives with the download stage,
            // which is where there is finally something worth reporting.
            if current == STEP_INSTALL
                && on_next.install.may_proceed()
                && let Err(reason) = Self::place()
            {
                on_next.set_notice(&catalog::install_failed(&reason), catalog::Tone::Warning);
                return Ok(());
            }
            // Leaving the last question is what records the answers. Here
            // rather than at Finish because the engine check comes after it and
            // runs for seconds: a user who closes the window while it is
            // working has still answered every question, and their shortcut
            // should be waiting for them.
            //
            // Rewritten on every pass rather than once, which matters because
            // Back works: a user who goes back from the engine check, unticks
            // a box and presses Next again has changed their answer, and a
            // write-once guard would keep the old one while showing the new.
            // The files are five short lines; there is nothing to save by
            // skipping them.
            if current == STEP_PRIVACY {
                let written = seed::write(&on_next.answers());
                if !written.all_recorded() {
                    // Not fatal, and not silent either. The install is complete
                    // and every one of these has a control in Settings, so the
                    // honest thing is to name what did not stick and carry on.
                    on_next.set_status(
                        &catalog::seeds_not_recorded(&written.failed),
                        catalog::Tone::Warning,
                    );
                }
            }
            if current + 1 < catalog::STEPS.len() {
                on_next.step.set(current + 1);
                on_next.show_step()?;
            } else {
                // Finish. Start the app, then close — README's own description
                // of setup ends "Launches the app", and until now it did not.
                //
                // Before closing rather than after: `close` destroys the window
                // and ends the message loop, so anything after it is running in
                // a process on its way out.
                if on_next.launch_installed_app() {
                    on_next.close()?;
                }
            }
            Ok(())
        });

        let on_cancel = self.clone();
        self.cancel.on().bn_clicked(move || {
            // Stop the transfer before the window goes. The bytes already
            // fetched stay on disk with their resume metadata, so this is a
            // pause rather than a discard — but the worker thread has to be told,
            // or it keeps writing into a directory while the process is trying
            // to exit.
            if let Ok(run) = on_cancel.run.try_borrow()
                && let Some(run) = run.as_ref()
            {
                run.cancel();
            }
            on_cancel.close()?;
            Ok(())
        });
    }

    /// The page's key line.
    fn set_key(&self, text: &str, tone: catalog::Tone) {
        say(&self.key, &self.key_tone, text, tone);
    }

    /// What a reporting step found.
    fn set_notice(&self, text: &str, tone: catalog::Tone) {
        say(&self.notice, &self.notice_tone, text, tone);
    }

    /// What a question step says back.
    fn set_status(&self, text: &str, tone: catalog::Tone) {
        say(&self.status, &self.status_tone, text, tone);
    }

    /// Start the transfer for this machine, once.
    ///
    /// Called on arrival at the download step rather than from a button, because
    /// the step exists to do exactly this and a wizard that waits to be told
    /// twice is a wizard people get stuck in. Re-entering the step after a
    /// completed or failed run does start a new one — which is the retry path,
    /// and it resumes rather than restarting.
    fn begin_transfer(&self) {
        if self
            .run
            .try_borrow()
            .is_ok_and(|run| run.as_ref().is_some_and(|run| !run.progress.finished()))
        {
            return; // Already in flight; leave it alone.
        }
        self.settled.set(false);
        self.ready.set(false);
        // The provider the compatibility step decided on.
        //
        // It selects nothing about the *model* — there is one Granite pack and
        // it is the CPU-variant GGUF either way, because the CUDA worker
        // offloads that same file. What the provider decides is whether the
        // GPU worker and its two CUDA libraries are fetched alongside it.
        let provider = self.machine.admissibility.preferred_provider();
        let (message, tone, show_bar) = match download::plan(provider) {
            Ok(plan) if plan.already_satisfied() => {
                // Nothing to transfer, and it must not be reported as a transfer
                // that finished instantly: what happened is that the files are
                // present and their digests still match.
                self.settled.set(true);
                self.ready.set(true);
                (
                    catalog::DOWNLOAD_ALREADY_PRESENT.to_owned(),
                    catalog::Tone::Good,
                    false,
                )
            }
            Ok(plan) => {
                let described = catalog::describe_download_plan(
                    &plan.items.iter().map(|item| item.label).collect::<Vec<_>>(),
                    plan.total_bytes,
                );
                *self.run.borrow_mut() = Some(download::start(plan));
                (described, catalog::Tone::Plain, true)
            }
            Err(failure) => {
                // No plan means nothing was attempted, which is a different
                // thing from a transfer that failed, and the copy says which.
                self.settled.set(true);
                (failure, catalog::Tone::Warning, false)
            }
        };
        self.set_notice(&message, tone);
        let _ = self
            .transfer
            .hwnd()
            .ShowWindow(if show_bar { co::SW::SHOW } else { co::SW::HIDE });
        self.apply_next_availability(STEP_DOWNLOAD);
    }

    /// Whether Next may be pressed on `index`.
    ///
    /// One place, because two of these gates now exist and they are applied from
    /// three call sites — arriving at a step, finishing a transfer, and coming
    /// back to a step that was already satisfied. Spread out, the third one is
    /// the one that gets forgotten, and the symptom is a Next button that stays
    /// dead after a download the user watched succeed.
    fn apply_next_availability(&self, index: usize) {
        self.next.hwnd().EnableWindow(match index {
            // A refused install is a dead end by design — the remaining steps
            // configure something that is not going to be written.
            STEP_INSTALL => self.install.may_proceed(),
            // Continuing without the models would install an app that cannot
            // transcribe, and the last step would then have to report a failure
            // that this step already knew about.
            STEP_DOWNLOAD => self.ready.get(),
            // A shortcut another program owns is not a shortcut. Unlike the
            // download there is always a way forward here — two other
            // combinations are on screen — so this gate can never trap someone.
            STEP_SHORTCUT => self.shortcut_free.get(),
            _ => true,
        });
    }

    /// Read the transfer and repaint, if one is running and its step is showing.
    /// Starts the engine check, replacing any previous verdict.
    ///
    /// Idempotent while one is in flight, so a second Retry click does not
    /// spawn a second worker and a second ~2 GB model load.
    fn begin_verify(&self) {
        if self
            .verify
            .try_borrow()
            .is_ok_and(|slot| slot.as_ref().is_some_and(|run| !run.progress().finished()))
        {
            return;
        }
        self.verify_settled.set(false);
        let root = probe::install_root();
        let Some(root) = root else {
            // The same refusal `place` gives. Reached only if the profile lost
            // LOCALAPPDATA between installing and this step, which is not a
            // condition to guess a path for.
            self.set_notice(catalog::INSTALL_ROOT_UNLOCATABLE, catalog::Tone::Warning);
            return;
        };
        let Some(model_root) = download::installed_model_root() else {
            self.set_notice(catalog::DATA_ROOT_UNLOCATABLE, catalog::Tone::Warning);
            return;
        };
        *self.verify.borrow_mut() = Some(smoke::start(smoke::staged_worker(&root), model_root));
        self.set_notice(catalog::SMOKE_RUNNING, catalog::Tone::Plain);
        let _ = self.retry.hwnd().EnableWindow(false);
    }

    /// Writes the engine check's verdict once it settles.
    fn poll_verify(&self) {
        if self.step.get() != STEP_VERIFY || self.verify_settled.get() {
            return;
        }
        let Ok(slot) = self.verify.try_borrow() else {
            return;
        };
        let Some(run) = slot.as_ref() else {
            return;
        };
        if !run.progress().finished() {
            return;
        }
        self.verify_settled.set(true);
        // A verdict that settled with nothing in the slot cannot happen --
        // `start` writes it before the flag -- but reporting success for it
        // would be the silent-pass this whole step exists to prevent, so an
        // absent verdict reads as the engine not having run.
        let (message, tone) = match run.progress().verdict() {
            Some(smoke::Verdict::Verified) => (catalog::SMOKE_VERIFIED, catalog::Tone::Good),
            Some(smoke::Verdict::Mismatch { .. }) => {
                (catalog::SMOKE_MISMATCH, catalog::Tone::Warning)
            }
            Some(smoke::Verdict::Unavailable { .. }) | None => {
                (catalog::SMOKE_UNAVAILABLE, catalog::Tone::Warning)
            }
        };
        self.set_notice(message, tone);
        self.retry.hwnd().EnableWindow(true);
    }

    fn poll_transfer(&self) {
        if self.step.get() != STEP_DOWNLOAD {
            return;
        }
        let Ok(slot) = self.run.try_borrow() else {
            return;
        };
        let Some(run) = slot.as_ref() else {
            return;
        };
        let progress = &run.progress;
        let done = progress.bytes().min(run.total_bytes);

        // Scaled through u128 rather than multiplying two u64s: 2.3 GB of bytes
        // times a thousand is comfortably inside u64, but the next artifact this
        // step gains need not be, and an overflow here would show a full bar
        // over an empty download.
        let filled = if run.total_bytes == 0 {
            0
        } else {
            u32::try_from(
                u128::from(done) * u128::from(TRANSFER_STEPS) / u128::from(run.total_bytes),
            )
            .unwrap_or(TRANSFER_STEPS)
        };
        self.transfer.set_position(filled);

        if progress.finished() {
            if self.settled.get() {
                return;
            }
            self.settled.set(true);
            if let Some(failure) = progress.failure() {
                self.ready.set(false);
                self.set_notice(&failure, catalog::Tone::Warning);
            } else {
                self.ready.set(true);
                self.transfer.set_position(TRANSFER_STEPS);
                self.set_notice(
                    &catalog::describe_download_complete(&run.labels, run.total_bytes),
                    catalog::Tone::Good,
                );
            }
            self.apply_next_availability(STEP_DOWNLOAD);
            return;
        }

        // Installing wins over verifying if both are somehow set: it is the
        // later phase, and reporting the earlier one would walk the message
        // backwards.
        let phase = if progress.installing() {
            catalog::Phase::Installing
        } else if progress.verifying() {
            catalog::Phase::Verifying
        } else {
            catalog::Phase::Transferring
        };
        self.set_notice(
            &catalog::describe_download_progress(
                &run.labels,
                progress.current(),
                done,
                run.total_bytes,
                phase,
            ),
            catalog::Tone::Plain,
        );
    }

    /// Place the payload and register the installation.
    fn place() -> Result<(), String> {
        // Held until `perform` returns: when setup carries its payload inside
        // its own executable, dropping this deletes the directory it reads from.
        let payload =
            payload::stage().map_err(|failure| catalog::describe_payload_failure(&failure))?;
        let root =
            probe::install_root().ok_or_else(|| catalog::INSTALL_ROOT_UNLOCATABLE.to_owned())?;
        install::perform(payload.directory(), &root)
    }

    /// Close the wizard.
    ///
    /// `DestroyWindow` rather than posting `WM_CLOSE`: `PostMessage` and
    /// `SendMessage` are `unsafe` in `winsafe` — they hand a raw payload to an
    /// arbitrary window procedure — and this workspace forbids `unsafe`
    /// outright. Destroying the main window is the safe equivalent; `winsafe`
    /// turns the resulting `WM_NCDESTROY` into the quit that ends `run_main`.
    fn close(&self) -> w::AnyResult<()> {
        self.window.hwnd().DestroyWindow()?;
        Ok(())
    }

    /// Paint the current step.
    ///
    /// One function for every navigation path, so Back and Next cannot drift
    /// into disagreeing about what a step looks like.
    fn show_step(&self) -> w::AnyResult<()> {
        let index = self.step.get();
        let Some(step) = catalog::STEPS.get(index) else {
            return Ok(());
        };
        // `SetWindowText` on the control's own handle, not `set_text_and_resize`:
        // that one sizes the label to its content, which would let a longer
        // translation silently overrun the controls beneath it. The layout is
        // fixed and the text has to live inside it.
        self.heading.hwnd().SetWindowText(step.heading)?;
        self.position
            .hwnd()
            .SetWindowText(&catalog::step_position(index))?;
        self.set_key(step.key, step.key_tone);
        self.body.hwnd().SetWindowText(step.body)?;
        // Which controls this step owns, before anything writes text: the
        // notice and the question controls share one band, so showing the wrong
        // set draws a report over a set of radio buttons.
        self.show_questions(index);
        // The steps that report, reporting. The download and engine-check steps
        // are absent on purpose: `begin_transfer` and `poll_verify` own their
        // text, because what to say depends on what they found, and deciding
        // that in two places would be two answers that can disagree. The
        // question steps are absent for the same reason one level along —
        // `show_questions` writes their status line.
        if !matches!(index, STEP_DOWNLOAD | STEP_VERIFY) && !asks_a_question(index) {
            let (message, tone) = match index {
                STEP_COMPATIBILITY => (
                    catalog::describe_machine(&self.machine),
                    catalog::Tone::Plain,
                ),
                STEP_INSTALL => catalog::describe_install_decision(&self.install),
                _ => (catalog::STEP_NOT_BUILT.to_owned(), catalog::Tone::Plain),
            };
            self.set_notice(&message, tone);
            self.transfer.hwnd().ShowWindow(co::SW::HIDE);
        }

        // Retry belongs to the engine check alone. Hidden rather than disabled
        // everywhere else: it sits outside the three navigation buttons, so
        // hiding it moves nothing the user is aiming at.
        self.retry.hwnd().ShowWindow(if index == STEP_VERIFY {
            co::SW::SHOW
        } else {
            co::SW::HIDE
        });
        self.progress
            .set_position(u32::try_from(index + 1).unwrap_or(1));

        // Back is meaningless on the first step, and the last step commits
        // rather than continues. Disabling beats hiding: a button that vanishes
        // moves the ones beside it, and the target the user aimed at changes
        // under the cursor.
        self.back.hwnd().EnableWindow(index > 0);
        // Disabled rather than removed wherever a step gates progress, so the
        // reason above it stays on screen and Back still works; the copy
        // explains what to do instead.
        self.apply_next_availability(index);
        self.next
            .hwnd()
            .SetWindowText(if index + 1 == catalog::STEPS.len() {
                catalog::FINISH
            } else {
                catalog::NEXT
            })?;
        // Arriving at the engine check starts it, once. Returning here with a
        // verdict already settled leaves it on screen rather than re-running a
        // ~2 GB model load because the user pressed Back and Next.
        if index == STEP_VERIFY && self.verify.try_borrow().is_ok_and(|slot| slot.is_none()) {
            self.begin_verify();
        }

        // Last, because it both writes the notice and sets Next: run earlier and
        // the generic paths above would overwrite what it decided.
        if index == STEP_DOWNLOAD {
            self.begin_transfer();
        }
        Ok(())
    }

    /// Show the controls this step owns and hide the rest.
    ///
    /// Every control exists from the first paint — `winsafe` gives no choice —
    /// so "which page am I on" is entirely a question of visibility, and it has
    /// to be answered for *all* of them on every navigation. Answering it in
    /// one place is what stops a control from a step the user left behind
    /// showing through the next one; the version of this that only showed the
    /// arriving step's controls left the previous step's radio buttons on top
    /// of the compatibility report.
    fn show_questions(&self, index: usize) {
        let visible = |shown: bool| if shown { co::SW::SHOW } else { co::SW::HIDE };
        let provider = index == STEP_PROVIDER;
        let shortcut = index == STEP_SHORTCUT;
        let privacy = index == STEP_PRIVACY;

        for button in self.provider.iter() {
            button.hwnd().ShowWindow(visible(provider));
        }
        for button in self.shortcut.iter() {
            button.hwnd().ShowWindow(visible(shortcut));
        }
        self.words.hwnd().ShowWindow(visible(index == STEP_WORDS));
        self.keep_transcripts.hwnd().ShowWindow(visible(privacy));
        self.disk_logging.hwnd().ShowWindow(visible(privacy));

        let asks = asks_a_question(index);
        self.notice.hwnd().ShowWindow(visible(!asks));
        self.status.hwnd().ShowWindow(visible(asks));
        if !asks {
            return;
        }
        // The status line, which is the only thing a question step reports.
        // Recomputed on arrival rather than remembered, because the shortcut's
        // answer can change while setup is open: another program can take the
        // combination between one visit to this step and the next.
        let (message, tone) = match index {
            STEP_PROVIDER => self.describe_provider_choice(),
            STEP_SHORTCUT => self.verify_selected_shortcut(),
            // The words page reports its own count, which is a fact about what
            // was typed rather than about the page being arrived at.
            STEP_WORDS => self.word_count_message(),
            _ => (String::new(), catalog::Tone::Plain),
        };
        self.set_status(&message, tone);
    }

    /// How many words the box will hand over, and whether that is none.
    ///
    /// Counted through [`seed::parse_vocabulary`], the same function that
    /// produces the seed, so this can never claim a number the file disagrees
    /// with. Its own message rather than a bare digit: "0 words" beside an empty
    /// box reads as a failure, and an empty list is a perfectly good answer here.
    fn word_count_message(&self) -> (String, catalog::Tone) {
        let typed = self.words.text().unwrap_or_default();
        let words = seed::parse_vocabulary(&typed);
        if words.is_empty() {
            (catalog::WORDS_NONE.to_owned(), catalog::Tone::Plain)
        } else {
            (catalog::words_counted(&words), catalog::Tone::Good)
        }
    }

    /// Repaint the word count, after the box changed.
    fn show_word_count(&self) {
        if self.step.get() != STEP_WORDS {
            return;
        }
        let (message, tone) = self.word_count_message();
        self.set_status(&message, tone);
    }

    /// Why the provider step offers what it offers.
    fn describe_provider_choice(&self) -> (String, catalog::Tone) {
        catalog::describe_provider_options(
            self.machine.admissibility.preferred_provider()
                == speakeasy_models::ExecutionProvider::Cuda,
            download::graphics_card_configuration_published(),
        )
    }

    /// Ask Windows whether the selected shortcut is actually free.
    ///
    /// By registering it, then letting it go. There is no way to look at a key
    /// combination and tell — the owner of a global shortcut is not discoverable
    /// — so the alternative to this is telling the user their choice is fine and
    /// letting them find out by pressing it and having nothing happen, which is
    /// the failure the step's own copy promises not to make.
    ///
    /// Registered against the wizard's window, which receives the resulting
    /// `WM_HOTKEY` and ignores it; the registration lives for microseconds.
    fn verify_selected_shortcut(&self) -> (String, catalog::Tone) {
        let choices = shortcut_choices();
        let Some(choice) = self
            .shortcut
            .selected_index()
            .and_then(|index| choices.get(index))
        else {
            // No selection is not a state this group can be in — one button is
            // selected at construction — but reporting free would be a claim
            // about a combination nobody named.
            self.shortcut_free.set(false);
            return (catalog::SHORTCUT_UNKNOWN.to_owned(), catalog::Tone::Warning);
        };
        let window = self.window.hwnd();
        let free = window
            .RegisterHotKey(PROBE_HOTKEY_ID, choice.modifiers, choice.key)
            .is_ok();
        if free {
            // Immediately. Holding it would make setup the owner of the
            // shortcut the app is about to ask for, so the app's own
            // registration would then fail — setup would have caused the
            // conflict it exists to detect.
            let _ = window.UnregisterHotKey(PROBE_HOTKEY_ID);
        }
        self.shortcut_free.set(free);
        self.apply_next_availability(STEP_SHORTCUT);
        if free {
            (
                catalog::shortcut_available(choice.binding),
                catalog::Tone::Good,
            )
        } else {
            (
                catalog::shortcut_taken(choice.binding),
                catalog::Tone::Warning,
            )
        }
    }

    /// Start the app setup just installed, and say whether it started.
    ///
    /// `false` keeps the wizard open with the reason on screen, which is the
    /// whole point of returning anything: closing regardless would leave a user
    /// who watched setup complete looking at an empty desktop, with the last
    /// thing they saw being a window that said dictation works.
    ///
    /// No `CREATE_NO_WINDOW` and no detaching. The app is a
    /// `windows_subsystem = "windows"` binary, so it allocates no console to
    /// hide, and this process has none to pass on — `relaunch_detached` gave
    /// the wizard null standard handles precisely so that anything it starts
    /// inherits nothing.
    fn launch_installed_app(&self) -> bool {
        let Some(executable) = install::installed_app_executable() else {
            self.set_notice(catalog::APP_NOT_FOUND, catalog::Tone::Warning);
            self.notice.hwnd().ShowWindow(co::SW::SHOW);
            return false;
        };
        match std::process::Command::new(&executable).spawn() {
            Ok(_) => true,
            Err(error) => {
                self.set_notice(
                    &catalog::app_did_not_start(&error.to_string()),
                    catalog::Tone::Warning,
                );
                self.notice.hwnd().ShowWindow(co::SW::SHOW);
                false
            }
        }
    }

    /// Everything the wizard collected, as the seed writer wants it.
    fn answers(&self) -> seed::Answers {
        let choices = shortcut_choices();
        let shortcut = self
            .shortcut
            .selected_index()
            .and_then(|index| choices.get(index))
            .map_or(choices[0].binding, |choice| choice.binding);
        seed::Answers {
            shortcut: shortcut.to_owned(),
            vocabulary: seed::parse_vocabulary(&self.words.text().unwrap_or_default()),
            keep_transcripts: self.keep_transcripts.is_checked(),
            disk_logging: self.disk_logging.is_checked(),
            provider: if self.provider.selected_index() == Some(0) {
                seed::Provider::GraphicsCard
            } else {
                seed::Provider::Processor
            },
        }
    }

    pub fn run(&self) -> w::AnyResult<i32> {
        self.window.run_main(None)
    }
}
