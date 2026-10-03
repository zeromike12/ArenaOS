# Third full-run startup-sampling discovery (unqualified)

Candidate `5ecc4aecc051e6a00bbf6833b6b29d8a7c8f1db7` detected all 12
production boundary mutants, then its restored GREEN failed because the first
visible desktop frame preceded the first complete native accounting sample.
The shared fixture indexed an empty sample list. The dispatcher was paused,
the active multi-child mutation control completed and restored exact source,
then the dispatcher was terminated. This incomplete run is invalid; it is
neither a full-suite result nor a qualification receipt.

This archive preserves the partial run log and original failure traceback.
No original failing serial capture is claimed: that shared file had already
been replaced by a later GREEN boot. The correction waits for a complete
native snapshot before any desktop workflow starts and accepts only complete
newline-terminated serial records. Every possible byte cut, including a split
UTF-8 glyph and partial final cap count, is checked on the host. Native counts,
exact teardown requirements and production authority code remain unchanged.
