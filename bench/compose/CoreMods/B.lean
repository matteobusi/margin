import CoreMods.A

theorem double_comm (a b : Nat) : double (a + b) = double (b + a) := by
  rw [double_add, double_add, Nat.add_comm]
