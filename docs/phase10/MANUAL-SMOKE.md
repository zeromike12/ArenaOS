# Michael's supplementary desktop smoke test

Run the qualified QEMU bundle using its included RUNNING instructions with a
visible display. A healthy empty desktop has a top bar, centered six-app dock
and pointer. Shell/serial remains a separate trusted administrative session.
Record the baseline commit/EFI hash, QEMU version and any observations. Manual
results supplement automated QMP, native accounting and durable-byte proof.

1. Click each actual dock application. Use F1..F6 with the pointer idle too.
   Check Terminal, Files, Editor, Settings, Monitor and Gallery appear as
   separate windows. A seventh launch must show a bounded refusal.
2. Click exposed window content/title bars repeatedly. Confirm the clicked
   window comes forward and its chrome becomes focused. Use F7 to cycle focus.
   Type in a focused Terminal or Editor; other windows must not receive text.
3. Drag every title bar repeatedly, including partly beyond each screen edge.
   Content moves with its window; a reachable title portion remains visible.
   Drag while another window is visible. Release outside a client and click a
   different control; the old client must not retain a pressed-button state.
4. In Terminal type `help`, `echo hello`, `ls`, `ps`, and
   `put user-note hello desktop`. Read it with `cat user-note`. Edit the command
   line with arrows, Home/End, backspace and Delete. Use Up/Down to view bounded
   scrollback. `launch gallery` starts a real second application. Unsupported
   commands show a useful result; this is a bounded ArenaOS session.
5. In Files select `user-note`, inspect its text and open it in Editor. Use New
   to create `user-new`. Creating `user-note` again must refuse and preserve its
   bytes. Delete `user-new` and refresh. AFS1 is flat; there are no invented
   directory breadcrumbs or folders.
6. In Editor insert text, newline/tab, move its cursor and delete characters.
   Save existing content, Save As `user-copy`, then reopen it. New clears a
   clean document. Dirty close offers Save/Discard/Cancel; Cancel preserves
   the dirty buffer and a second close still works. Errors retain the buffer.
7. Change light/dark theme in Settings with other windows open. Chrome, dock,
   components and app content must repaint coherently. Disable/enable motion.
   Reboot the same scratch disk; the choices must persist. Display information
   reflects the actual mode. No unsupported hardware switches are shown.
8. Keep Monitor visible while opening/closing apps. Real process and region
   counts change. Use arrows to scroll process rows. Labels/PIDs are descriptive.
9. Toggle Gallery's local light/dark reference with `t`, focus/unfocus it and
   compare reusable controls and states. This does not change durable settings.
10. Close all apps using chrome and F8; reopen them repeatedly. Leave the
    desktop idle for at least ten minutes, then repeat typing, dragging and
    launch/close. Confirm serial commands still work and shutdown is clean.

Record exact actions, expected/observed behavior and screenshot names for
failures. Do not treat a manual pass as a substitute for the qualification suite.
