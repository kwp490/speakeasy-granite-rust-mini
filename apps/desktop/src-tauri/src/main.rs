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
// `proof-mode` is excluded deliberately: `run_proof_mode` reports through
// `eprintln!` and exit codes for the installed smoke entry points, and those need
// somewhere for stderr to land. No script builds that feature today, so the
// shipped binary is always the windowed one.
#![cfg_attr(
    all(not(debug_assertions), not(feature = "proof-mode")),
    windows_subsystem = "windows"
)]

fn main() {
    #[cfg(feature = "proof-mode")]
    if run_proof_mode() {
        return;
    }
    speakeasy_desktop::run();
}

#[cfg(feature = "proof-mode")]
fn run_proof_mode() -> bool {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| argument == "--phase1-installed-smoke")
    {
        if let Err(error) = speakeasy_desktop::run_phase1_installed_smoke() {
            eprintln!("phase1_installed_smoke={error}");
            std::process::exit(1);
        }
        return true;
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--phase2-installed-smoke")
    {
        let Some(archive) = arguments.get(index + 1) else {
            eprintln!("phase2_installed_smoke=archive_required");
            std::process::exit(2);
        };
        let Some(root) = arguments.get(index + 2) else {
            eprintln!("phase2_installed_smoke=root_required");
            std::process::exit(2);
        };
        if let Err(error) = speakeasy_desktop::run_phase2_installed_smoke(
            std::path::Path::new(archive),
            std::path::Path::new(root),
        ) {
            eprintln!("phase2_installed_smoke={error}");
            std::process::exit(1);
        }
        return true;
    }
    false
}
