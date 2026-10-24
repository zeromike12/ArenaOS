# Fresh-extraction package negative controls

These checks ran against the archive extracted to `/tmp/phase15-patched-tcg-review-f54797f-20261024-v3`, outside the development checkout. Each case stores the exact command, combined output, exit status, and machine-readable result. The missing-evidence case uses a fresh hard-linked copy so removing one directory entry cannot change the verified extraction. Wrong identity manifests are copied outside the package. The semantic case uses the recorded unpatched strict serial log with QEMU exit explicitly passed as zero. The accelerator case uses the included wrapper that changes only the QMP initialization smoke to KVM; it never launches ArenaOS. These are host/mock negative controls; they do not establish Windows or hardware guest execution.

{
  "cases": {
    "accelerator-initialization-failure": "wrapper advertised the pinned QEMU identity, then substituted unavailable KVM only in initialization smoke; preflight failed as expected",
    "missing-evidence": "rejected missing strict serial log before receipt verification",
    "semantic-failure-qemu-exit-zero": "receipt recorded qemu_exit_zero=true and qualification_pass=false",
    "wrong-efi-identity": "rejected changed strict_arenaos_image.efi_sha256; manifest value was bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e, test value was 64 zeroes",
    "wrong-emulator-identity": "rejected changed emulator.binary_sha256; manifest value was e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff, test value was 64 zeroes",
    "wrong-esp-identity": "rejected changed strict_arenaos_image.esp_sha256; manifest value was 631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f, test value was 64 zeroes"
  },
  "environment": "Linux host execution of tools against freshly extracted package; accelerator case uses the committed mock wrapper; no Windows or hardware guest execution is claimed.",
  "overall": "PASS; every negative control was rejected as intended",
  "schema_version": 1
}

## Packaging assembly history

The initial archive extraction checks caught two packaging-only defects before
publication: nested `manifest.json` files had been omitted from the top-level
hash map, and the overlay omitted `verify_phase15_xm_controls.py`. Both were
fixed, the final archive rebuilt, and the final fresh extraction passed. Exact
outputs and corrections are recorded in `assembly-iterations.json`. No guest
source, emulator patch, or qualification result changed.
