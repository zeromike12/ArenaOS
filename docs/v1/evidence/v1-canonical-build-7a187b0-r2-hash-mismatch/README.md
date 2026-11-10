# Historical canonical-build refusal — 7a187b0, r2

This run is a preserved **FAIL**, not a qualification result. The protected build stopped before producing EFI/ESP because the Phase-14 PIE rebuild generated SHA-256 `a65f3a1e9bc29158b509e5c267cbbacf53ae6ba0a39b2b5972f1cad56e086c05`, while the committed fixture guard requires `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`. The guard was not changed, no development build was substituted, and no guest stages ran.

The generated ELF and original build/receipt logs are retained here. Byte strings show its source path resolves under `/home/user/ArenaOS`, while the committed fixture carries `/workspace/ArenaOS`; this is an observed path-identity mismatch, not a source attribution. The next attempt is being prepared with a true canonical bind mount, without changing source, guard, expected hash, or timing.
