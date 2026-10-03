# Frozen archive verification

The archive passed all checksum checks and a strict, independent graphical
boot. `independent-extracted-boot.txt` and `extracted-evidence/` preserve that
actual result. Run `python3 VERIFY-EXTRACT.py` to independently repeat it, or
extract the archive and run `python3 phase10_archive_boot.py` in its directory.
The latter needs only Python's standard library, QEMU and the bundled files.

The source-frozen bundle helper successfully validated the 97-suite source,
exact image/100 receipt and screenshots, then created the archive. Its isolated
Python launcher left the extraction-directory argument in `sys.argv`, so the
packaged script refused its CLI before boot. `packaging-launcher-discovery.txt`
preserves the failure. The corrected invocation consumes that host argument,
exactly as the already-qualified archive-preflight fixture does. Guest code,
packaged scripts, oracle requirements and qualified EFI were unchanged.

`VERIFY-EXTRACT.py` records the corrected host invocation for this checkpoint.
The one-line source helper correction below belongs in Sol's next source
qualification; Opus should leave `tools/` unchanged during the design pass:

```python
runner = 'import runpy,sys;sys.path.insert(0,sys.argv.pop(1));runpy.run_path(sys.path[0]+"/phase10_archive_boot.py",run_name="__main__")'
```

This is an archive-automation limitation, not a failed stability boot. The
completed exact-EFI loop passed 100/100 without failures/retries. The independent
archive attempt was restarted with the correct command and passed actual pixels,
input, process/native accounting, network/device checks and clean shutdown.
