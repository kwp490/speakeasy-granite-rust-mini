#![allow(clippy::needless_pass_by_value)]

mod capture_wizard;
mod granite_engine;
mod native_catalog;
mod runtime_wizard;

// These sections intentionally share one private namespace. The split is a
// source-organization boundary, not a visibility or behavior change: commands
// and coordinators still use the same private helpers and DTOs they used before.
include!("views.rs");
include!("coordinators.rs");
include!("coordinators/runtime.rs");
include!("commands/profile.rs");
include!("commands/models.rs");
include!("commands/capture.rs");
include!("commands/dictation.rs");
include!("composition.rs");
include!("tests.rs");
