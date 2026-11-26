def double (n : Nat) : Nat := n + n

/-- A private helper: its proof must stay hidden. -/
theorem double_add (a b : Nat) : double (a + b) = double a + double b := by
  unfold double; omega
