use alloc::string::String;
use margin_canon::{Arena, Decl};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelError(pub String);

/// A Lean 4 kernel type checker.
///
/// Implementations must be deterministic.
///
/// Implementations:
/// - `margin-kernel-nanoda`: nanoda_lib (Rust), vendored in `third_party/`.
/// - `lean4lean`: later. Needs Lean's C output and runtime compiled for riscv32.
pub trait Kernel {
    /// Name and version, written to the journal (informational).
    fn name(&self) -> &str;

    /// Type-check the `user` declarations, in order, in an environment that already
    /// contains the `env` declarations. `env` is trusted (its integrity comes from
    /// `env_root`) and must not be re-checked.
    fn check(&mut self, arena: &Arena, env: &[Decl], user: &[Decl]) -> Result<(), KernelError>;
}

/// DANGER: accepts every declaration. Pipeline tests only.
#[cfg(feature = "unsound-skip-typecheck")]
#[derive(Default)]
pub struct UncheckedKernel;

#[cfg(feature = "unsound-skip-typecheck")]
impl Kernel for UncheckedKernel {
    fn name(&self) -> &str {
        "UNSOUND-unchecked"
    }
    fn check(&mut self, _: &Arena, _: &[Decl], _: &[Decl]) -> Result<(), KernelError> {
        Ok(())
    }
}
