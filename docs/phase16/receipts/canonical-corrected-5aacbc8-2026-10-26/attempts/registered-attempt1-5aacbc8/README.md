# Registered attempt 1 from clean 5aacbc8

- Outer command exited 1 after 6543 seconds: 2 of 121 suites failed.
- Failed suite markers: `test_m84_crash.py` and `test_m9_client_death.py`.
- `test_m84_crash.py` failed only the final `policy/close` recovery check after all prior crash-prefix and policy-boundary checks passed. The preserved recovery serial shows `pkg: INELIGIBLE` and clean shutdown but does not contain `packaged READY`, indicating the expected recovered-state conjunction failed on that marker; no product cause is inferred from this alone.
- The preserved `serial-m9-client-death.log` was overwritten by the subsequent `test_m9_client_death_red.py` restored GREEN invocation of the same production suite, which passed. Therefore it is not the failing first boot's serial and is retained only to document that overwrite hazard; the full-suite log records `GUEST FAILURE ... fatal [arena ERROR halt]` followed by the production assertion.
- Focused reruns must be retained separately. A focused pass does not convert this failed complete run into a pass; a later complete registered run is still required.
