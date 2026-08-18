//! The wizard window.
//!
//! Native Win32 through `winsafe`, not a Tauri window: the thing that provisions
//! `WebView2` is the thing this replaces, and repair mode has to draw on a
//! machine where something is already broken. See
//! `docs/handoff/setup-wizard-redesign.md`.
//!
//! This is the chrome — the window, the step sequence, and the navigation
//! between them. Each step's own controls arrive with the stage that implements
//! it, and until then the step says so. Building the frame first is deliberate:
//! the order and the wording are the part worth reviewing before any of it can
//! download three gigabytes.
//!
//! Runs in a process with no console, launched by `relaunch_detached`. That
//! matters more than it looks: a console window belonging to `SpeakEasy` becomes
//! the delivery target for a dictation, and the last step of this wizard runs a
//! real one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use winsafe::prelude::*;
use winsafe::{self as w, co, gui};

use crate::{catalog, download, install, probe};

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
    pub const BODY_TOP: i32 = 74;
    /// The step's own explanation. Short on purpose — the space below it
    /// belongs to what the step actually found or is asking for.
    pub const BODY_HEIGHT: i32 = 110;
    pub const NOTICE_TOP: i32 = 194;
    /// Where a step's findings and controls go. Sized for the longest thing it
    /// currently has to hold: the compatibility report, which runs to about
    /// eleven lines on a machine with a graphics card whose engines disagree.
    pub const NOTICE_HEIGHT: i32 = 170;
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
    body: gui::Label,
    notice: gui::Label,
    transfer: gui::ProgressBar,
    progress: gui::ProgressBar,
    back: gui::Button,
    next: gui::Button,
    cancel: gui::Button,
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
}

/// Indices into [`catalog::STEPS`] for the steps that have content.
///
/// Named rather than spelled as numbers at the point of use: the step order is
/// still settling, and a bare `3` in `show_step` would silently start describing
/// the wrong step the first time one is inserted above it.
const STEP_COMPATIBILITY: usize = 0;
const STEP_DOWNLOAD: usize = 2;
const STEP_INSTALL: usize = 3;

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
        let heading = gui::Label::new(
            &window,
            gui::LabelOpts {
                text: "",
                position: gui::dpi(layout::MARGIN, layout::HEADING_TOP),
                size: gui::dpi(content_width, layout::HEADING_HEIGHT),
                ..Default::default()
            },
        );
        let position = gui::Label::new(
            &window,
            gui::LabelOpts {
                text: "",
                position: gui::dpi(layout::MARGIN, layout::POSITION_TOP),
                size: gui::dpi(content_width, layout::POSITION_HEIGHT),
                ..Default::default()
            },
        );
        let body = gui::Label::new(
            &window,
            gui::LabelOpts {
                text: "",
                position: gui::dpi(layout::MARGIN, layout::BODY_TOP),
                size: gui::dpi(content_width, layout::BODY_HEIGHT),
                ..Default::default()
            },
        );
        let notice = gui::Label::new(
            &window,
            gui::LabelOpts {
                text: catalog::STEP_NOT_BUILT,
                position: gui::dpi(layout::MARGIN, layout::NOTICE_TOP),
                size: gui::dpi(content_width, layout::NOTICE_HEIGHT),
                ..Default::default()
            },
        );
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

        // Right-aligned, in reading order from the right edge: Cancel, Next,
        // Back. Windows convention, and the one the user's other installers use.
        let right = layout::WINDOW.0 - layout::MARGIN;
        let cancel = Self::button(&window, catalog::CANCEL, right - layout::BUTTON.0);
        let next = Self::button(
            &window,
            catalog::NEXT,
            right - (layout::BUTTON.0 * 2) - layout::BUTTON_GAP,
        );
        let back = Self::button(
            &window,
            catalog::BACK,
            right - (layout::BUTTON.0 * 3) - (layout::BUTTON_GAP * 2),
        );

        let wizard = Self {
            window,
            heading,
            position,
            body,
            notice,
            transfer,
            progress,
            back,
            next,
            cancel,
            step: Rc::new(Cell::new(0)),
            machine: Rc::new(probe::run()),
            install: Rc::new(install::decide_now()),
            run: Rc::new(RefCell::new(None)),
            settled: Rc::new(Cell::new(false)),
            ready: Rc::new(Cell::new(false)),
        };
        wizard.wire_events();
        wizard
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

        let on_tick = self.clone();
        self.window.on().wm_timer(POLL_ID, move || {
            on_tick.poll_transfer()?;
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
                on_next
                    .notice
                    .hwnd()
                    .SetWindowText(&catalog::install_failed(&reason))?;
                return Ok(());
            }
            if current + 1 < catalog::STEPS.len() {
                on_next.step.set(current + 1);
                on_next.show_step()?;
            } else {
                // Finish. Closing is all there is to do until the steps
                // themselves land; it must not claim setup succeeded.
                on_next.close()?;
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
        let (message, show_bar) = match download::plan(provider) {
            Ok(plan) if plan.already_satisfied() => {
                // Nothing to transfer, and it must not be reported as a transfer
                // that finished instantly: what happened is that the files are
                // present and their digests still match.
                self.settled.set(true);
                self.ready.set(true);
                (catalog::DOWNLOAD_ALREADY_PRESENT.to_owned(), false)
            }
            Ok(plan) => {
                let described = catalog::describe_download_plan(
                    &plan.items.iter().map(|item| item.label).collect::<Vec<_>>(),
                    plan.total_bytes,
                );
                *self.run.borrow_mut() = Some(download::start(plan));
                (described, true)
            }
            Err(failure) => {
                // No plan means nothing was attempted, which is a different
                // thing from a transfer that failed, and the copy says which.
                self.settled.set(true);
                (failure, false)
            }
        };
        let _ = self.notice.hwnd().SetWindowText(&message);
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
            _ => true,
        });
    }

    /// Read the transfer and repaint, if one is running and its step is showing.
    fn poll_transfer(&self) -> w::AnyResult<()> {
        if self.step.get() != STEP_DOWNLOAD {
            return Ok(());
        }
        let Ok(slot) = self.run.try_borrow() else {
            return Ok(());
        };
        let Some(run) = slot.as_ref() else {
            return Ok(());
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
                return Ok(());
            }
            self.settled.set(true);
            if let Some(failure) = progress.failure() {
                self.ready.set(false);
                self.notice.hwnd().SetWindowText(&failure)?;
            } else {
                self.ready.set(true);
                self.transfer.set_position(TRANSFER_STEPS);
                self.notice
                    .hwnd()
                    .SetWindowText(&catalog::describe_download_complete(
                        &run.labels,
                        run.total_bytes,
                    ))?;
            }
            self.apply_next_availability(STEP_DOWNLOAD);
            return Ok(());
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
        self.notice
            .hwnd()
            .SetWindowText(&catalog::describe_download_progress(
                &run.labels,
                progress.current(),
                done,
                run.total_bytes,
                phase,
            ))?;
        Ok(())
    }

    /// Place the payload and register the installation.
    fn place() -> Result<(), String> {
        let payload =
            install::payload_directory().ok_or_else(|| catalog::PAYLOAD_UNLOCATABLE.to_owned())?;
        install::perform(&payload, &probe::install_root())
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
        self.body.hwnd().SetWindowText(step.body)?;
        // Built steps report; the rest say they are unbuilt. As each stage lands
        // its own arm appears here and the fallback shrinks.
        // Built steps report; the rest say they are unbuilt. As each stage lands
        // its own arm appears here and the fallback shrinks. The download step
        // is absent on purpose: `begin_transfer` below owns its text, because
        // what to say depends on whether there is anything to fetch, and
        // deciding that twice would be two answers that can disagree.
        if index != STEP_DOWNLOAD {
            self.notice.hwnd().SetWindowText(&match index {
                STEP_COMPATIBILITY => catalog::describe_machine(&self.machine),
                STEP_INSTALL => catalog::describe_install_decision(&self.install),
                _ => catalog::STEP_NOT_BUILT.to_owned(),
            })?;
            self.transfer.hwnd().ShowWindow(co::SW::HIDE);
        }
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
        // Last, because it both writes the notice and sets Next: run earlier and
        // the generic paths above would overwrite what it decided.
        if index == STEP_DOWNLOAD {
            self.begin_transfer();
        }
        Ok(())
    }

    pub fn run(&self) -> w::AnyResult<i32> {
        self.window.run_main(None)
    }
}
