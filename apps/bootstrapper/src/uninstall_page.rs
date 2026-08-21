//! The page that asks what an uninstall should remove.
//!
//! Native Win32 through `winsafe`, like [`mod@crate::wizard`] and for the same
//! reason: the uninstaller runs on a machine where the app is being taken away,
//! so it cannot depend on anything the app provisioned.
//!
//! # Why this replaced a message box
//!
//! `uninstall.rs` has specified one page with a checkbox per removable item
//! since 2026-08-15, and it was never built. What stood in for it was a single
//! Yes/No `MessageBox` naming the whole scope in its body — honest, but not a
//! choice: it offered everything or nothing, so a user who wanted to keep their
//! transcript history had no answer except a command-line flag meant for tests.
//!
//! # There is exactly one question
//!
//! No confirmation follows this page. [`ask`]'s `Remove` button *is* the
//! confirmation — it is the default button, it is focused, and the sentence
//! immediately above it says the action cannot be undone. A second dialog
//! re-asking what this page just asked is the "sequential prompts answered
//! blind" shape this module's sibling header warns about, and it teaches people
//! that the way through an uninstaller is to press Enter twice.
//!
//! # The boxes default to removing
//!
//! Owner decision, 2026-08-21, and the same one that inverted the command line:
//! an uninstall leaves nothing unless told otherwise. [`Removals::default`]
//! still selects nothing, because that is the API's default for a caller who
//! forgot to ask; this page has asked.
//!
//! # A window here is a delivery-target candidate
//!
//! Anything this process puts in the foreground can become the target
//! `deliver_final_text` pastes into. Safe for the same reason
//! [`crate::repair`]'s dialog is: an uninstall refuses outright while the app is
//! running, so there is no dictation in flight to hijack.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use winsafe::prelude::*;
use winsafe::{self as w, co, gui};

use crate::catalog;
use crate::uninstall::{Removable, Removals};

/// Layout, in DPI-independent units.
///
/// Grouped rather than inlined for the reason [`mod@crate::wizard`]'s own `layout`
/// gives: these numbers are relative to each other, and a change to one that
/// skips the others looks right at 100% and overlaps at 250%.
mod layout {
    /// How many check-box rows to reserve.
    ///
    /// Written out rather than cast from `Removable::ALL.len()`: a `usize` to
    /// `i32` cast is two pedantic lints for a value that is five, and a layout
    /// constant is not worth an allow. `the_layout_reserves_a_row_for_every_item`
    /// fails if the two ever disagree, which is the property the cast was for.
    pub const CHECK_ROWS: i32 = 5;

    pub const MARGIN: i32 = 16;
    pub const HEADING_TOP: i32 = 16;
    pub const HEADING_HEIGHT: i32 = 26;
    pub const INTRO_TOP: i32 = 50;
    pub const INTRO_HEIGHT: i32 = 20;
    pub const CHECK_TOP: i32 = 78;
    /// One check box. Sized for the text rather than the glyph — at 250% a
    /// 20 px row clips this font's descenders, which is the measurement the
    /// wizard's own `CONTROL_ROW` records.
    pub const CHECK_ROW: i32 = 24;
    /// Where the list of files setup did not place goes.
    ///
    /// Measured on this machine at 250%: the heading plus three staged CUDA
    /// libraries wraps to four lines of a 41 px cell, 164 px inside the 240 px
    /// this reserves -- so one spare line, not the two the arithmetic suggests,
    /// because the heading itself takes one. A longer list clips rather than
    /// overlapping the sentence below it.
    pub const UNRECOGNISED_TOP: i32 = CHECK_TOP + CHECK_ROW * CHECK_ROWS + MARGIN;
    pub const UNRECOGNISED_HEIGHT: i32 = 96;
    pub const IRREVERSIBLE_TOP: i32 = UNRECOGNISED_TOP + UNRECOGNISED_HEIGHT + 8;
    pub const IRREVERSIBLE_HEIGHT: i32 = 20;
    pub const BUTTON_TOP: i32 = IRREVERSIBLE_TOP + IRREVERSIBLE_HEIGHT + MARGIN;
    pub const BUTTON: (i32, i32) = (96, 28);
    pub const BUTTON_GAP: i32 = 8;
    pub const WINDOW: (i32, i32) = (480, BUTTON_TOP + BUTTON.1 + MARGIN);
}

/// The page, and the answer it is collecting.
#[derive(Clone)]
struct Page {
    window: gui::WindowMain,
    boxes: Rc<Vec<gui::CheckBox>>,
    irreversible: gui::Label,
    remove: gui::Button,
    cancel: gui::Button,
    /// `None` until Remove is pressed. Closing the window any other way — the
    /// title bar, Alt+F4, Cancel — leaves it `None`, which [`ask`] reads as
    /// "remove nothing".
    ///
    /// A `RefCell<Option<..>>` rather than a flag plus a second read of the
    /// controls: the controls are destroyed with the window, so the answer has
    /// to be taken while they still exist.
    answer: Rc<RefCell<Option<Removals>>>,
    /// Set once the window has been shown, so the focus is only forced on the
    /// first paint. After that the focus belongs to wherever the user tabbed.
    focused: Rc<Cell<bool>>,
}

/// Ask what to remove, and return the answer.
///
/// `None` means remove nothing and stop — Cancel, the close box, or a window
/// that could not be created. **That last case matters**: a page nobody saw is
/// not consent, and treating a failure to draw as agreement is the
/// silent-success shape this repository keeps finding. It is the same reasoning
/// the message box this replaces used for a `MessageBox` that returned an error.
///
/// `unrecognised` is the files in `proof/` that setup did not put there, from
/// [`crate::uninstall::unrecognised_proof_files`], asked *before* anything is
/// deleted so the page can name them.
pub fn ask(unrecognised: &[String]) -> Option<Removals> {
    let page = Page::new(unrecognised);
    // A window that could not run is not an answer. `run_main` returns the
    // message loop's exit code; anything that stopped it before the Remove
    // handler recorded a choice leaves `answer` as it started.
    if page.window.run_main(None).is_err() {
        return None;
    }
    let answer = page.answer.borrow();
    *answer
}

impl Page {
    fn new(unrecognised: &[String]) -> Self {
        let window = gui::WindowMain::new(gui::WindowMainOpts {
            title: catalog::UNINSTALL_WINDOW_TITLE,
            size: gui::dpi(layout::WINDOW.0, layout::WINDOW.1),
            // No resize and no maximize, matching the wizard: the content is a
            // fixed list at a fixed measure and a resizable window here only
            // creates a layout problem.
            style: gui::WindowMainOpts::default().style | co::WS::MINIMIZEBOX,
            ..Default::default()
        });
        let content_width = layout::WINDOW.0 - (layout::MARGIN * 2);
        let label = |top: i32, height: i32, text: &str| {
            gui::Label::new(
                &window,
                gui::LabelOpts {
                    text,
                    position: gui::dpi(layout::MARGIN, top),
                    size: gui::dpi(content_width, height),
                    ..Default::default()
                },
            )
        };
        let _heading = label(
            layout::HEADING_TOP,
            layout::HEADING_HEIGHT,
            catalog::UNINSTALL_HEADING,
        );
        let _intro = label(
            layout::INTRO_TOP,
            layout::INTRO_HEIGHT,
            catalog::UNINSTALL_INTRO,
        );
        let boxes = Self::check_boxes(&window, content_width);
        let _unrecognised = label(
            layout::UNRECOGNISED_TOP,
            layout::UNRECOGNISED_HEIGHT,
            &unrecognised_text(unrecognised),
        );
        let irreversible = label(
            layout::IRREVERSIBLE_TOP,
            layout::IRREVERSIBLE_HEIGHT,
            catalog::UNINSTALL_IRREVERSIBLE,
        );
        let [remove, cancel] = Self::button_row(&window);
        let page = Self {
            window,
            boxes: Rc::new(boxes),
            irreversible,
            remove,
            cancel,
            answer: Rc::new(RefCell::new(None)),
            focused: Rc::new(Cell::new(false)),
        };
        page.wire_events();
        page
    }

    /// One check box per [`Removable`], in the order that enum declares them.
    ///
    /// Iterated from `Removable::ALL` rather than written out, so a sixth thing
    /// the uninstaller learns to remove cannot become a thing this page forgets
    /// to offer. That property is why the enum exists at all — its own doc
    /// comment says so.
    fn check_boxes(window: &gui::WindowMain, content_width: i32) -> Vec<gui::CheckBox> {
        Removable::ALL
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let index = i32::try_from(index).unwrap_or(0);
                gui::CheckBox::new(
                    window,
                    gui::CheckBoxOpts {
                        text: &label_for(*item),
                        position: gui::dpi(
                            layout::MARGIN,
                            layout::CHECK_TOP + layout::CHECK_ROW * index,
                        ),
                        size: gui::dpi(content_width, layout::CHECK_ROW),
                        // Checked, all of them. The owner's decision of
                        // 2026-08-21: an uninstall leaves nothing unless the
                        // user says otherwise, and this page is where they say
                        // it.
                        check_state: co::BST::CHECKED,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    /// Remove and Cancel, right-aligned, Cancel outermost.
    ///
    /// Cancel is in the rightmost slot because that is where the wizard puts it
    /// and where Windows puts it, so the button a user reaches for to back out
    /// is in the place their hand already goes. Remove carries
    /// `BS::DEFPUSHBUTTON` — it is the default and it takes the focus.
    fn button_row(window: &gui::WindowMain) -> [gui::Button; 2] {
        let right = layout::WINDOW.0 - layout::MARGIN;
        let slot =
            |index: i32| right - (layout::BUTTON.0 * index) - (layout::BUTTON_GAP * (index - 1));
        let button = |text: &str, left: i32, style: co::BS| {
            gui::Button::new(
                window,
                gui::ButtonOpts {
                    text,
                    position: gui::dpi(left, layout::BUTTON_TOP),
                    width: gui::dpi_x(layout::BUTTON.0),
                    height: gui::dpi_y(layout::BUTTON.1),
                    control_style: style,
                    ..Default::default()
                },
            )
        };
        [
            button(catalog::UNINSTALL_REMOVE, slot(2), co::BS::DEFPUSHBUTTON),
            button(
                catalog::UNINSTALL_KEEP_EVERYTHING,
                slot(1),
                co::BS::PUSHBUTTON,
            ),
        ]
    }

    fn wire_events(&self) {
        // `BS::DEFPUSHBUTTON` makes Remove the *default* button and does not
        // make it the *focused* one, and the owner's decision of 2026-08-21 was
        // about what happens when somebody presses Enter without reading.
        // Measured both ways rather than assumed: with this line removed the
        // focus lands on the heading static -- not on the first check box, and
        // not on either button. Forced on the first paint only, so that after
        // it the focus belongs to wherever the user tabbed.
        let on_show = self.clone();
        self.window.on().wm_show_window(move |_| {
            if !on_show.focused.replace(true) {
                let _ = on_show.remove.hwnd().SetFocus();
            }
            Ok(())
        });

        // The one line on this page that is not a list item. Coloured here
        // because a static paints its own text and asks its parent for the ink
        // on the way; see `wizard::say` for the same pairing and why the
        // background mode has to be transparent.
        let on_color = self.clone();
        self.window.on().wm_ctl_color_static(move |p| {
            if p.hwnd == *on_color.irreversible.hwnd() {
                let _ = p.hdc.SetTextColor(w::COLORREF::from_rgb(0x9b, 0x1c, 0x1c));
            }
            let _ = p.hdc.SetBkMode(co::BKMODE::TRANSPARENT);
            Ok(w::HBRUSH::GetSysColorBrush(co::COLOR::BTNFACE)?)
        });

        let on_remove = self.clone();
        self.remove.on().bn_clicked(move || {
            // Read while the controls still exist. Destroying the window
            // destroys them, so an answer taken afterwards would be taken from
            // handles that are gone.
            let mut selected = Removals::default();
            for (item, control) in Removable::ALL.iter().zip(on_remove.boxes.iter()) {
                selected.select(*item, control.is_checked());
            }
            *on_remove.answer.borrow_mut() = Some(selected);
            on_remove.window.hwnd().DestroyWindow()?;
            Ok(())
        });

        // Cancel records nothing, which `ask` reads as remove nothing. The
        // close box and Alt+F4 take the same path by doing nothing at all.
        let on_cancel = self.clone();
        self.cancel.on().bn_clicked(move || {
            on_cancel.window.hwnd().DestroyWindow()?;
            Ok(())
        });
    }
}

/// A check box's text: the item's label, and for the models a measured size.
///
/// Only the models entry carries a figure, and it carries a measured one. See
/// [`catalog::removable_label_with_size`] for why one and not five, and
/// [`crate::uninstall::measure`] for why measured and not written down.
fn label_for(item: Removable) -> String {
    match (item, crate::uninstall::measure(item)) {
        (Removable::Models, Some(bytes)) => catalog::removable_label_with_size(item.label(), bytes),
        _ => item.label().to_owned(),
    }
}

/// The unrecognised-files block, or nothing.
///
/// An empty label rather than a hidden one: the space is the same either way at
/// this window size, and a control that is sometimes absent is a control whose
/// absence has to be laid out around.
fn unrecognised_text(unrecognised: &[String]) -> String {
    if unrecognised.is_empty() {
        return String::new();
    }
    format!(
        "{}\r\n  {}",
        catalog::UNINSTALL_UNRECOGNISED,
        unrecognised.join("\r\n  ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every removable item gets a box, and the page reads them positionally.
    ///
    /// `wire_events` zips `Removable::ALL` against the controls, so the two
    /// lists agreeing in length and order is what makes the answer the user's.
    /// A sixth variant added without a sixth control would silently never be
    /// selectable, and this is the assertion that stops it -- the window itself
    /// cannot be built in a test, but the invariant that matters is about the
    /// lists, not the pixels.
    #[test]
    fn the_layout_reserves_a_row_for_every_item() {
        assert_eq!(
            usize::try_from(layout::CHECK_ROWS).expect("a row count fits a usize"),
            Removable::ALL.len(),
            "a sixth removable item needs a sixth row"
        );
    }

    #[test]
    fn the_page_offers_one_choice_per_removable_item() {
        let labels: Vec<String> = Removable::ALL.iter().map(|item| label_for(*item)).collect();
        assert_eq!(labels.len(), Removable::ALL.len());
        for (item, rendered) in Removable::ALL.iter().zip(labels.iter()) {
            assert!(
                rendered.starts_with(item.label()),
                "a label must still name its item: {rendered}"
            );
        }
    }

    /// A size is appended to the models entry and to nothing else.
    ///
    /// Asserted against the rendering rather than against `measure`, because
    /// the decision being pinned is a presentation one: four kilobyte figures
    /// beside the one that matters is what this avoids.
    #[test]
    fn only_the_models_entry_may_carry_a_size() {
        for item in Removable::ALL {
            if item == Removable::Models {
                continue;
            }
            assert_eq!(
                label_for(item),
                item.label(),
                "only the models entry names a size"
            );
        }
    }

    /// The size is measured and formatted, not guessed.
    #[test]
    fn a_measured_size_is_rendered_beside_the_label() {
        assert_eq!(
            catalog::removable_label_with_size("Downloaded speech models", 2_140_000_000),
            "Downloaded speech models (2.1 GB)"
        );
        // Under a gigabyte the smaller unit takes over, so a partially
        // downloaded set does not read as `0.1 GB`.
        assert_eq!(
            catalog::removable_label_with_size("Downloaded speech models", 140_000_000),
            "Downloaded speech models (140 MB)"
        );
    }

    /// Nothing in `proof/` means no list, not an empty heading.
    #[test]
    fn unrecognised_files_are_named_only_when_there_are_some() {
        assert!(unrecognised_text(&[]).is_empty());
        let listed = unrecognised_text(&["cublas64_13.dll".to_owned()]);
        assert!(listed.starts_with(catalog::UNINSTALL_UNRECOGNISED));
        assert!(listed.contains("cublas64_13.dll"));
    }
}
