# Reserved-byte test sensitivity

Final tested source: 5aacbc88c50d4565a7bb9f4075be27e40df27779
Previous production base: feaa9b189cb9ed0ac2d078011aa0335385622407

- previous-c.log: the final C host tests linked against only the previous
  production streams.c. It exits nonzero with exactly eight failures: both
  reserved endpoints in each of the three rings and both reserved-tail
  endpoints. The corrected production code passes the same host suite in
  ../host/host-canonical.log (1721 checks).
- previous-rust.log: the final Rust streams tests in a detached previous
  tree with only the production tail scan, exact-page length gate, and
  initializer tail zeroing reverted in scratch. It exits nonzero with the
  three new tests failing and the 24 pre-existing tests passing. The
  corrected production code passes 27/27 in ../host/host-canonical.log.
