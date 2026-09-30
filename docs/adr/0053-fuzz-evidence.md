# ADR-0053 coverage-guided host artifact fuzz (2026-09-30)

**Scope:** Host-only, deterministic mutation fuzz of the frozen v1 package/policy byte parsers **and** the exact vendored Rust verifier. The parser campaign is *coverage-guided* by Python decoder line-transition feedback; each parser-accepted candidate with a known signer is sent to the **exact-source, native serial-backend Rust Ed25519 verifier** (`audit/phase84-crypto/src/bin/verify_wire.rs`), with periodically sampled independent OpenSSL comparisons. A second campaign uses **LLVM source-region instrumentation of that Rust binary and all compiled dependencies** for independent coverage-guided mutation feedback. Neither is a guest service or production-key claim. The checked-in [`28-input parser corpus`](../../audit/phase84-crypto/corpus/) and [`11-input Rust corpus`](../../audit/phase84-crypto/crypto-corpus/) contain only signed public test artifacts/malformed variants, never seeds or private keys. `tools/test_package_record.py` independently covers 10,000 seeded mutations and policy-chain/exhaustion cases.

Run from repository root after `tools/dev-env/bootstrap.sh` and `. tools/dev-env/env.sh`:

```sh
(cd audit/phase84-crypto && cargo build --offline --locked --release --target x86_64-unknown-linux-gnu --bin verify_wire)
python3 tools/fuzz_phase84_records.py --iterations 10000 --corpus-in audit/phase84-crypto/corpus
(cd audit/phase84-crypto && CARGO_TARGET_DIR="$PWD/../../build/phase84-coverage-target" \
  RUSTFLAGS='--cfg curve25519_dalek_backend="serial" -C instrument-coverage' \
  cargo build --offline --locked --target x86_64-unknown-linux-gnu --bin verify_wire)
python3 tools/fuzz_phase84_crypto.py --rounds 500 --corpus-in audit/phase84-crypto/crypto-corpus
```

`fuzz_phase84_records.py` pins its PRNG seed, parses both formats under per-input decoder line-transition tracing, retains inputs with newly reached transitions, feeds length/truncation/bit/byte/insert/delete/splice/boundary/integer mutations, and enforces fixed resource/iteration bounds. It verifies all initial valid signed fixtures, refuses any mutated artifact that would otherwise pass crypto, and records parser-accepted signature-only mutations which fail cryptography. It also root-signs a test policy containing a **noncanonical, non-small-order subordinate public-key alias**: the Python structural parser and OpenSSL root signature accept it, while the Rust oracle must reject the subordinate's canonicality. For unknown signer-key IDs there is no key to consult; those cases remain negative and cannot become successful by guessing a file name. Corpus filenames embed truncated SHA-256; the replay tool checks them before use.

**Observed replay (10,000 iterations per format, fixed seed):**

| Format | Decoder transitions | Corpus | Parser refusals | Parsed-but-rejected | Vendored verifier calls | OpenSSL samples |
|---|---:|---:|---:|---:|---:|---:|
| Package | 25 | 9 | 9,093 | 835 | 703 | 9 |
| Policy | 34 | 19 | 9,465 | 456 | 535 | 7 |

**LLVM-instrumented Rust replay:** 500 bounded inputs, **2,964 unique executed source regions** across the exact native serial verifier closure, 11 coverage-retained corpus inputs, 14 valid positive executions, **zero executed `ed25519-dalek/src/signing.rs` source regions**. `llvm-profdata merge` + `llvm-cov export` give Rust-specific feedback after each case; mutated key/message/signature/Ed25519 scalar/point and policy subordinate bytes drive subsequent corpus selection. This is source-region, not all-branch or bare-metal guest coverage. Inputs include RFC vectors 1/2 and OpenSSL-signed package/policy messages.

**A real harness hardening finding:** LLVM feedback drove a mutant that replaced the *separately supplied* subordinate public key in the Rust policy oracle while leaving root-signed policy bytes intact; a naive oracle would check the substitute instead of the signed header. The Rust host oracle now requires its subordinate parameter to equal bytes 48..80 of the exact signed policy header, and its SHA-256 to equal the header key ID before canonical-key/strict-signature checks. This is bounded hardening of the **existing** approved signed-field binding, not a wire-format/trust-model change. The guest decoder must bind these bytes itself and must not trust an out-of-band policy key.

No mutated artifact was accepted as a new valid signed object. A deliberate always-accept verifier negative control made the parser fuzz test fail at package iteration 2 with `unexpected crypto accept/refusal`; this is not a test that can pass when signature verification is bypassed. Existing host tests also check RFC 8032 vectors 1/2, wrong root/domain, noncanonical scalar, small-order key/point, and 512 signature bit mutations. The audit-only no_std library compiles offline for the bare-metal target using the same vendored sources and features.

**Limitations and continuation:** Neither Python line transitions nor LLVM Rust source regions prove exhaustive arithmetic/path coverage, memory safety, all possible 4,288-byte inputs, or actual fsd/IPC/service state. Guest parser, visible-record rescan, recipient marker validation, persistence and QEMU tests remain required *after* ADR acceptance. Any changed wire/feature/target requires a new campaign and source audit. The public test seed is used only by offline OpenSSL host fixture generation; it must not enter the EFI, ESP or guest disk.
