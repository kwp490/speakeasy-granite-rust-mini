//! What UI Automation actually offers in whatever window is focused when this
//! runs.
//!
//! Live external typing needs to track the exact range `SpeakEasy` inserted, and
//! that is only possible if the target exposes readable document offsets — which
//! Electron apps frequently do not. Answering it empirically is what decides the
//! adapter design, so run it against each target you care about before writing
//! any of it.
//!
//! ```text
//! cargo run -p speakeasy-windows --example focused_target_probe
//! ```
//!
//! You have ten seconds after starting it to click into the target's text box
//! and type a couple of words.
//!
//! It was a `#[test] #[ignore]`d as "interactive probe" in `speakeasy-desktop`
//! until 2026-08-28. It asserts nothing — every outcome prints and returns — so
//! carrying it as a test inflated the ignored count with something no run could
//! fail, and it needed `--nocapture` or the whole report was swallowed, which is
//! the recorded hazard of measuring into captured stdout. As an example it
//! prints to a terminal by default and needs no flags.
//!
//! It also moved crates. It lived in the desktop crate's test module while using
//! nothing from it: `TargetObserver` is this crate's, and this is where the
//! instrument belongs.

use std::time::Duration;

use speakeasy_domain::SessionId;
use speakeasy_windows::TargetObserver;

fn main() {
    let observer = match TargetObserver::spawn() {
        Ok(observer) => observer,
        Err(error) => {
            println!("could not start the UI Automation observer: {error:?}");
            return;
        }
    };
    println!("\nFocus the target's text field and type a few words. Probing in 10s...");
    std::thread::sleep(Duration::from_secs(10));

    let snapshot = match observer.inspect(SessionId::from_bytes([0x70; 16])) {
        Ok(snapshot) => snapshot,
        Err(refusal) => {
            println!("REFUSED: {refusal:?}");
            return;
        }
    };
    println!("app            : {}", snapshot.executable.path);
    println!("integrity      : {:?}", snapshot.integrity);
    println!("capability     : {:?}", snapshot.capability);
    println!("read_only      : {}", snapshot.is_read_only);
    println!("password       : {}", snapshot.is_password);
    println!(
        "patterns       : text={} text2(caret)={} value={}",
        snapshot.patterns.text, snapshot.patterns.text2, snapshot.patterns.value
    );
    match &snapshot.selection {
        Some(selection) => println!(
            "selection      : start={:?} end={:?} caret={:?} empty={}",
            selection.start, selection.end, selection.caret, selection.is_empty
        ),
        None => println!("selection      : NONE"),
    }
    println!(
        "content f.print: {}",
        if snapshot.content_fingerprint.is_some() {
            "readable"
        } else {
            "NONE"
        }
    );

    // The verdict that actually decides the design.
    let offsets = snapshot
        .selection
        .as_ref()
        .is_some_and(|selection| selection.start.is_some() && selection.end.is_some());
    println!(
        "\nVERDICT: {}",
        if offsets {
            "document offsets readable - an insertion range can be tracked and \
             verified, so a real select-and-replace is possible here."
        } else if snapshot.patterns.text {
            "TextPattern present but NO usable offsets - the inserted range \
             cannot be verified. Append-only with a refuse-to-correct fallback."
        } else {
            "no TextPattern - blind typing only. Nothing can be verified or \
             corrected in place; this target must stay commit-on-finish."
        }
    );
}
