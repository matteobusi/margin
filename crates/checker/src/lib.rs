//! The checking pipeline shared by the zkVM guest and the in-browser pre-check.
//!
//! [`run`] verifies environment inclusion proofs, type-checks user declarations with a
//! [`Kernel`], enforces the axiom policy, computes the statement cone and builds the
//! [`Journal`](margin_journal::Journal).

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

pub mod env;
mod input;
mod kernel;
pub mod policy;
mod run;

pub use env::{Env, EnvEntry, Opening};
pub use input::{DepOpening, ProverInput};
pub use kernel::{Kernel, KernelError};
#[cfg(feature = "unsound-skip-typecheck")]
pub use kernel::UncheckedKernel;
pub use run::{public_statement, run, run_profiled, CheckError, Origin, Output, RunResult};
