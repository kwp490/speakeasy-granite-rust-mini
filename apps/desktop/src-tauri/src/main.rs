// Without this the shipped binary is a *console* subsystem executable, and
// Windows allocates a console for it on every launch. Measured against the
// installed 1.2.2 build: its PE subsystem was 3 (console), and launching it
// produced a `CASCADIA_HOSTING_WINDOW_CLASS` Windows Terminal window titled with
// the full exe path, which stayed for the life of the app.
//
// That is not merely untidy. The console window is visible and takes the
// foreground, and delivery inspects the foreground window to decide where the
// transcript goes — so the first dictation after launch aimed at a terminal
// rather than at whatever the user was typing in. It is the same failure as the
// hidden settings window taking focus, from a second cause, and it was found by
// the same dictation proof.
//
// Unconditional in a release build. It used to be conditioned on the
// `proof-mode` feature as well, so a smoke-test build could keep its console
// for `eprintln!` output. That feature is gone -- no script ever built it --
// and an arm for a configuration nobody can select only obscured which binary
// actually ships.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    speakeasy_desktop::run();
}
