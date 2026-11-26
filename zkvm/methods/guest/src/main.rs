//! The margin guest: check a Lean development and commit the public journal.

#![no_main]

use margin_checker::{run_profiled, ProverInput};
use risc0_zkvm::guest::env;

risc0_zkvm::guest::entry!(main);

#[cfg(not(any(feature = "kernel-nanoda", feature = "unsound-skip-typecheck")))]
compile_error!("select a kernel feature: `kernel-nanoda` or `unsound-skip-typecheck`");

#[cfg(all(feature = "kernel-nanoda", feature = "unsound-skip-typecheck"))]
compile_error!("select exactly one kernel feature");

/// Log the cycle count at a phase boundary (goes to the host's stdout; a few lines only).
fn mark(phase: &str) {
    env::log(&format!("[cycles {:>12}] {phase}", env::cycle_count()));
}

fn main() {
    mark("start");
    // Raw framed read (length, then bytes): far cheaper than serde for a byte vector.
    let mut len: u32 = 0;
    env::read_slice(core::slice::from_mut(&mut len));
    let mut input = vec![0u8; len as usize];
    env::read_slice(&mut input);
    mark("read input");
    let input = ProverInput::decode(&input).expect("malformed prover input");
    mark("decoded input");

    #[cfg(feature = "kernel-nanoda")]
    let mut kernel = margin_kernel_nanoda::NanodaKernel { phase_hook: Some(mark) };
    #[cfg(feature = "unsound-skip-typecheck")]
    let mut kernel = margin_checker::UncheckedKernel;

    // A failed check panics: the prover gets no receipt.
    let journal = run_profiled(&input, &mut kernel, &mark).expect("check failed");
    mark("checker done");
    env::commit_slice(&journal.encode());
    mark("committed");
}
