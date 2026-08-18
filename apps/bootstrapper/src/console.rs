//! Getting repair-mode output to a caller from a GUI-subsystem binary.
//!
//! The bootstrapper is `windows_subsystem = "windows"` because it draws a
//! window: a console-subsystem process gets a terminal window it did not ask
//! for, and `CLAUDE.md` records that as one of three separate causes of the same
//! silent bug — any window `SpeakEasy` puts in the foreground becomes the
//! delivery target for a dictation, so the wizard's own console would hijack the
//! test dictation the wizard runs. It does not error; delivery refuses with
//! `target_inspect_refused` and falls back to the clipboard, which reads as a bug
//! somewhere else entirely.
//!
//! The cost is that a GUI-subsystem process starts with no console, so the repair
//! verbs — which `docs/RUNBOOK.md` documents as ordinary command-line commands —
//! have nowhere obvious to print. Three cases, and they are genuinely different:
//!
//! 1. **The caller redirected stdout** (`$out = & bootstrapper verify ...`, a
//!    pipe, a file). The handle is inherited and valid, and `println!` reaches
//!    it. This is what `scripts/Test-InstallerLifecycle.ps1` does.
//! 2. **The caller has a console** (`bootstrapper verify ...` typed into a
//!    terminal). Nothing is redirected, so the process starts with no valid
//!    stdout, and `AttachConsole` is what connects it to the caller's.
//! 3. **No console at all** (double-clicked). Nothing printed can ever be seen.
//!
//! Whether case 2 leaves `println!` working is a platform detail this module
//! does not reason about: `ensure_attached` reports what it actually observed,
//! and `main` decides. Verified empirically 2026-08-15 rather than from the
//! documentation, because "the output silently went nowhere" is indistinguishable
//! from "the command printed nothing", and this project has been bitten by
//! exactly that shape of failure before.

use winsafe::co;
use winsafe::prelude::*;
use winsafe::{AttachConsole, HSTD, PidParent};

/// Where repair-mode output can actually go.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
    /// A valid stdout the caller gave us — redirected, or a console we attached
    /// to that also rewired the standard handles. `println!` is enough.
    Stream,
    /// No writable stdout. Anything printed is lost, so the caller has to be
    /// told some other way.
    None,
}

/// Connect to the caller's console if there is one, and report where output can
/// go afterwards.
///
/// Called before the first write, because Rust resolves the standard-output
/// handle once and caches it: attaching afterwards would leave `println!`
/// pointing at the handle this process started with, which is the invalid one.
pub fn ensure_attached() -> Destination {
    if has_writable_stdout() {
        // Already redirected by the caller. Attaching would be wrong here as
        // well as unnecessary — it would point output at a console the caller
        // deliberately redirected away from.
        return Destination::Stream;
    }
    // `PidParent::Parent` is the console of whatever launched us. It fails when
    // there is none (a double-click), which is not an error worth reporting:
    // the re-check below is the answer either way.
    let _ = AttachConsole(PidParent::Parent);
    if has_writable_stdout() {
        Destination::Stream
    } else {
        Destination::None
    }
}

/// Whether this process has a standard-output handle worth writing to.
///
/// `GetStdHandle` succeeding is not sufficient on its own — a GUI-subsystem
/// process can hand back a null handle — so this asks the handle to describe
/// itself and treats "cannot answer" as unusable.
fn has_writable_stdout() -> bool {
    let Ok(mut guard) = HSTD::GetStdHandle(co::STD_HANDLE::OUTPUT) else {
        return false;
    };
    // `leak`, and it is load-bearing. `GetStdHandle` does not duplicate: it hands
    // back the process's own standard-output handle, and `CloseHandleGuard` would
    // close it on drop. Merely *asking whether* output works would then be what
    // broke it, and every later `println!` would fail — a check that destroys the
    // thing it measures, reported as a clean pass.
    let handle = guard.leak();
    if handle == HSTD::NULL || handle == HSTD::INVALID {
        return false;
    }
    // A console handle answers `GetConsoleMode`; a redirected file or pipe does
    // not, but does accept a zero-byte write. Either answer means writable, and
    // a zero-byte write appends nothing to what the caller is capturing.
    handle.GetConsoleMode().is_ok() || handle.WriteFile(&[]).is_ok()
}
