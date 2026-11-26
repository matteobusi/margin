//! Axiom policy.

/// Axioms that a claim may use.
pub const ALLOWED_AXIOMS: &[&str] = &["propext", "Quot.sound", "Classical.choice"];

/// Constants that make a claim invalid, even if the kernel accepts the term.
pub const FORBIDDEN: &[&str] = &[
    "sorryAx",             // `sorry`
    "Lean.ofReduceBool",   // `native_decide`
    "Lean.ofReduceNat",    // native Nat reduction
    "Lean.trustCompiler",  // trusts the compiler
];
