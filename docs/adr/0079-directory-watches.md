# ADR-0079 — Directory watches

Status: accepted (Phase 11 completion). Host GREEN/RED is
`tools/test_watch_red.py`. The guest proof is `tools/test_m11_watch.py`.

## Problem

The accepted Phase-11 goals require that an open folder show changes
without the user acting. Before this decision, Files re-listed a folder
only on focus, interaction or request. The broker re-read
`/Users/user/Desktop` once a second on its uptime tick. A change made by
another program (the Terminal) stayed invisible in an unfocused Files
window. The desktop surface could lag by up to a second, and the broker
polled filesd every second whether anything had changed or not.

The mechanism must add no authority. A path is presentation and must
never name what is watched. It must also leave no way to learn about a
directory the watcher cannot list.

## Decision

**A watch hangs off a held directory capability.** `OP_WATCH` is a
request on the client's own filesd record (a badged file capability,
ADR-0077) that names a directory. filesd refuses it:

* without `R_LIST` (`S_DENIED`);
* when the record is not a directory (`S_NOTDIR`).

The client lends one of its own notifications with the request (kind 3,
WRITE) and names a badge bit 0..=63. Anything else is `S_INVAL`. `/System`
stays outside user authority: no user lineage holds a record for it, so
there is nothing to watch it through.

**Bounded, refused before anything is kept.** filesd keeps at most
`WATCHES` = 24 watches, and one lineage keeps at most `LINEAGE_WATCHES`
= 4. The table reserves an entry before the lent notification is kept,
so a refusal (`S_FULL`) keeps nothing. If the record already has a watch,
the new one replaces it in place and the old lent notification is
dropped.

**What a change signals.** Each mutation names the directories it
changes:

| Mutation | Directories signalled |
|---|---|
| create, mkdir, unlink | the directory it ran in |
| rmdir | the parent; the removed directory's own watchers learn it is gone |
| write, truncate | the file's parent (size and time show in the listing) |
| rename | the source parent, and the destination parent when it differs (`watch::affected`) |

filesd ORs the watch's bit into the lent notification and counts an event
on the watch. The badge is only a hint, because a notification word
merges. The client confirms through its own record: `OP_WATCHED` returns
the event count (cleared) and whether the directory is gone, and only that
answer is authoritative.

**Exact object identity.** A watch stores the AFS2 object id, which is
`generation << 32 | index`, and matches it exactly (`watch::same`). A
removed directory's index comes back with a new generation, so a reused
index never signals the old watcher. The old watcher keeps its `gone` fact
and nothing more.

**Lifetime.** A watch ends with its record:

* `OP_UNWATCH`;
* the record's release;
* `OP_REVOKE`;
* lineage retirement, which is how process death reaches filesd through
  the broker's revoke.

filesd logs "N directory watch(es) ended with the lineage". If the lent
notification is dead when signalled, filesd removes that watch.

**Clients.**
* **Files** watches the folder it shows. It re-watches on every
  navigation, which replaces the watch in place. When its badge bit
  arrives, it asks `OP_WATCHED`: events re-list the folder, and a gone
  folder returns to home.
* **The broker** watches `/Users/user/Desktop` on badge bit 3 of its own
  clock notification. The one-second Desktop poll is removed. A watch
  wake marks the scene dirty, and the next render confirms through
  `OP_WATCHED` before re-reading.

**A notification capability carries no generation.** The kernel's
`CapObj::Notification` names an index, and adding a generation would
touch 75 kernel sites. This is mitigated, not hidden:

* The lent notifications are the clients' private clocks, which the broker
  creates per session slot.
* A watch dies with its lineage, so the session's process is gone before
  its slot is reused.
* The broker clears the slot's clock (`SYS_TRY_WAIT`) before handing it
  to the next session.
* A stray bit can only cause an extra `OP_WATCHED`, which returns
  nothing.

## Consequences

* An unfocused Files window updates for:
  * a create;
  * a rename into the folder;
  * a contents change;
  * a rename out of the folder;
  * removal of the folder itself.

  The desktop shows a file the Terminal wrote, with no poll and no click.
  Thirty folder switches never exhaust filesd, and closing Files ends its
  watch (`test_m11_watch.py`).
* RED controls (`test_watch_red.py`) mutate the production table:
  * matching only the index lets a reused index signal the old watcher
    (`a_reused_directory_index_never_signals_the_old_watcher` FAILS);
  * dropping the destination parent loses rename notifications
    (`a_rename_changes_both_parents` FAILS).

  Source is restored byte-exactly.
* filesd holds at most 24 more lent capabilities. Its 64-slot capability
  space keeps room for the per-request landed and minted caps.
