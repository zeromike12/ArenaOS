# Qualification attempts and retries

- Targeted attempt 1 at 8c472a2: Native/C1, C2, FP, and Notification passed
  for GCC and Clang. Live stdin failed before boot for both compilers because
  its test-only legacy-v1 transformed streams.c copy changed RING_STRIDE to
  832 but still hit the production ASTR-v2 exact 3328 tail static assert.
  Focused reproductions are targeted-attempt1-8c472a2-stdin-{gcc,clang}.log.
  Commit d5874b7 taught only that legacy-fixture transform to drop the v2-only
  assert; production streams.c was not weakened.
- Registered attempt 1 at d5874b7: tools/run_tests.sh exited 1 after 8181
  seconds with "1 of 121 test suites FAILED". The sole failing block was
  cargo fmt --check on two assertions in the new Rust test. A focused exact
  reproduction is registered-attempt1-d5874b7-rustfmt.log. Commit 5aacbc8 is
  that formatting-only correction. The original full attempt log was held in
  external sandbox evidence and was lost in a later sandbox reset; the final
  qualification below was rerun rather than relabelled.
- Registered attempt 1 at final source 5aacbc8 exited 1 after 6543 seconds
  with 2 of 121 suites failed (`test_m84_crash.py` and
  `test_m9_client_death.py`). Its complete log, diagnostic serials, and
  focused passing reruns are preserved in `registered-attempt1-5aacbc8/`.
  Those focused passes were not treated as a complete-suite pass. A second
  complete registered run at the same clean source then passed all 121
  suites in 6574 seconds; see `../registered/`.
- Final qualification at 5aacbc8 supersedes the earlier candidate attempts
  for corrected-candidate review, but does not erase or relabel any failed
  attempt. Earlier d314d20 receipts under `../../canonical-2026-10-26/`
  remain historical and unchanged.
