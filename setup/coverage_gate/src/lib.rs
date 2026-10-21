//! Coverage gates for the leptoskit workspace.
//!
//! CI runs the workspace tests once under `cargo llvm-cov` on Linux and the
//! UIKit and Android verifiers instrumented on a simulator and an emulator,
//! exports an lcov tracefile from each, and hands them to the `coverage_gate`
//! binary, which merges them and holds the result to two rules:
//!
//! - the workspace line coverage may not fall below the floor committed in
//!   `setup/coverage_gate/floor.txt`, which is raised as coverage improves;
//! - the lines a pull request adds must be covered at least as much as the
//!   patch minimum, so new code arrives with its tests.

pub mod diff;
pub mod gate;
pub mod lcov;
pub mod source;
