# Interrupted Phase-10 discovery run

Source: eeb7808e48bded741cd4182713e6719a0d1942d6.
This run is invalid: one signed-app test failed, and the runner was interrupted
before completing the historical suites. No full-suite result is claimed.

The checksummed archive preserves the partial complete-run log, the signed
application serial receipt and actual QMP captures. The final native sample
preceded the last Process retirement; shutdown was requested before the next
periodic accounting sample. The test now explicitly waits for fresh exact
baseline records/processes/regions/pages/maps/caps before allowing shutdown.
That wait still fails a real leak rather than guessing cleanup from a string.
The production lifecycle code is unchanged by this synchronization correction.
