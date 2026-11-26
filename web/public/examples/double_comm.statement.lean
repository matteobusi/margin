-- Lean v4.36.0, no imports (core only).
def double (n : Nat) : Nat := n + n
theorem double_comm (a b : Nat) : double (a + b) = double (b + a) := sorry
