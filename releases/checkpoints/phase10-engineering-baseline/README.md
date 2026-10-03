# Phase-10 Sol engineering baseline

Qualified executable/test/tool source: `11a45c34deae9a7a539a99fd7d3d6ea196ea6025`.
Full historical + Phase-10 suite: **97/97**. Fresh exact-image graphical
stability: **100/100**, zero failures/retries. EFI SHA-256:
`18357914fbfca7c7af87bd846c0f12f645e7f9a78d4816395d688b295ba90c7f`.

The archive is `arenaos-phase10-engineering-baseline-qemu-x86_64.tar.gz`;
verify its sibling `.sha256` file. The bundled standalone script boots only
extracted files and validates real dock/process/input/drag/close/relaunch/native
accounting, historical devices/wire behavior and serial shutdown. Its strict
independent extracted-boot receipt is preserved beside the archive.
Independent strict extracted graphical boot passed. Archive SHA-256:
`86c4853269b8e3475b205191ff6a80c20f096afe3609ee9dd049042b628592f8`.
Use `python3 VERIFY-EXTRACT.py` for a fresh independent check. `PACKAGING.md`
documents the corrected host invocation and the frozen bundle-helper limitation.

`SOURCE-AND-IMAGE.json`, full-suite/100-boot/static receipts,
`100-graphical-receipts.json`, `screenshot-manifest.json` and `screenshots/`
preserve exact measured evidence. The checksummed archive includes all three
earlier invalid-run discovery receipts and production RED controls.

Design contract: `docs/phase10/DESIGN-HANDOFF.md`, `UI-CAPABILITIES.md`,
`APP-CONTRACTS.md`, `DESIGN-REQUESTS.md` and `MANUAL-SMOKE.md`.
This is Sol's coherent reference skin. Opus owns the next design branch; final
Phase-10 visual qualification and Sol's integration review remain outstanding.
No manual Michael smoke-test result is claimed.
