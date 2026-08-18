//! Transcribes one WAV with Granite Speech and prints the result verbatim.
//!
//! This exists so that a fixture's ground truth is *discovered* rather than
//! assumed. `apps/bootstrapper`'s engine smoke test compares a whole transcript
//! against a pinned string, and a pinned string somebody typed from what they
//! believed the model would say is a test of that belief. Run this, read the
//! output, and paste exactly what came back.
//!
//! It earned its keep immediately: the first fixture said "and Granite writes
//! it down", and the model returned "Granit". The second said "dog, and
//! Monday" and the model chose "dog. And Monday" -- a punctuation decision, not
//! an error, and not one anybody would have typed from memory.
//!
//! ```text
//! cargo run --release -p speakeasy-granite --example transcribe_file -- <model.gguf> <mmproj.gguf> <audio.wav> [threads]
//! ```
//!
//! Release matters: a debug build of llama.cpp is slow enough to look hung.

use std::path::Path;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (model, projector, audio, threads) = match arguments.as_slice() {
        [model, projector, audio] => (model, projector, audio, None),
        [model, projector, audio, threads] => (model, projector, audio, Some(threads)),
        _ => {
            eprintln!("usage: transcribe_file <model.gguf> <mmproj.gguf> <audio.wav> [threads]");
            eprintln!(
                "got {} argument(s). A path with a space in it must be quoted -- this \
                 repository's own path has one.",
                arguments.len()
            );
            std::process::exit(2);
        }
    };

    let mut options = speakeasy_granite::GraniteOptions::default();
    // Overridable because the thread count is not a performance knob here: it
    // reproducibly changes greedy decode at 16, so a fixture pinned by whole
    // transcript is only safe if it is identical across the range a real
    // machine can pick. `recommended_thread_count` is
    // `(available_parallelism / 2).clamp(1, 8)`, so that range is 1..=8, and
    // this is how it gets swept rather than assumed.
    if let Some(threads) = threads {
        options.n_threads = threads
            .parse()
            .unwrap_or_else(|_| panic!("threads must be an integer, got {threads:?}"));
    }

    let started = std::time::Instant::now();
    match speakeasy_granite::transcribe_wav_file(
        Path::new(model),
        Path::new(projector),
        Path::new(audio),
        &options,
    ) {
        Ok(text) => {
            // Delimited, because leading and trailing whitespace is exactly the
            // kind of difference a whole-transcript comparison fails on and a
            // reader's eye skips over.
            println!("elapsed: {:?}", started.elapsed());
            println!("threads: {}", options.n_threads);
            println!(">>>{text}<<<");
        }
        Err(error) => {
            eprintln!("failed at {:?}: {}", error.stage(), error.detail());
            std::process::exit(1);
        }
    }
}
