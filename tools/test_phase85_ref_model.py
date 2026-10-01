#!/usr/bin/env python3
"""Phase 8.5 design-only Image ref-conservation model; NOT kernel/guest proof.

The transition ledger is deliberately separate from the independent scanner:
the scanner reads only the capspaces, queue fields, and in-flight pin owners.
The kernel hooks must later be audited/guest-tested against this oracle.
"""
from dataclasses import dataclass, field
from collections import Counter


@dataclass
class Call:
    state: str = 'Empty'
    caller: int = 0
    server: int = 0
    send_cap: int | None = None
    reply_cap: int | None = None
    landed: int | None = None


@dataclass
class Machine:
    live: set[int] = field(default_factory=set)
    retiring: set[int] = field(default_factory=set)
    refs: Counter = field(default_factory=Counter)
    pins: Counter = field(default_factory=Counter)
    spaces: dict[int, list[int | None]] = field(default_factory=lambda: {1: [None]*32, 2: [None]*32, 3: [None]*32})
    calls: list[Call] = field(default_factory=lambda: [Call() for _ in range(4)])
    pin_owners: dict[str, int] = field(default_factory=dict)
    checks: int = 0
    skip_hook: str = ''

    def hook(self, op: str, id_: int, delta: int) -> None:
        if id_ in self.live and self.skip_hook != op:
            self.refs[id_] += delta
            assert self.refs[id_] >= 0, (op, id_)

    def check(self, label: str) -> None:
        # NO registry.refs, hook, or transition helper is called here.
        oracle = Counter()
        for space in self.spaces.values():
            for id_ in space:
                if id_ in self.live:
                    oracle[id_] += 1
        for call in self.calls:
            for id_ in (call.send_cap, call.reply_cap):
                if id_ in self.live:
                    oracle[id_] += 1
        actual_pins = Counter(self.pin_owners.values())
        for id_ in self.live:
            assert oracle[id_] == self.refs[id_], (label, id_, oracle[id_], self.refs[id_])
            assert oracle[id_] > 0, (label, 'stranded LIVE ID')
        for id_ in self.retiring:
            assert id_ not in self.live and actual_pins[id_] > 0, (label, 'retired pin')
        for id_ in set(self.pins) | set(actual_pins):
            assert actual_pins[id_] == self.pins[id_], (label, 'pins', id_)
        self.checks += 1

    def mint(self, id_: int, pid: int, slot: int) -> None:
        assert id_ not in self.live and self.spaces[pid][slot] is None
        self.live.add(id_)
        self.spaces[pid][slot] = id_
        self.hook('mint', id_, 1)
        self.check('mint')

    def copy(self, src: int, a: int, dest: int, b: int) -> None:
        id_ = self.spaces[src][a]
        assert id_ is not None and self.spaces[dest][b] is None
        self.spaces[dest][b] = id_
        self.hook('copy', id_, 1)
        self.check('copy')

    def move(self, src: int, a: int, dest: int, b: int) -> None:
        assert self.spaces[dest][b] is None
        self.spaces[dest][b], self.spaces[src][a] = self.spaces[src][a], None
        self.check('move, no zero-ref gap')

    def drop(self, pid: int, slot: int) -> None:
        id_ = self.spaces[pid][slot]
        assert id_ is not None
        self.spaces[pid][slot] = None
        self.hook('drop', id_, -1)
        self.retire_if_unreferenced(id_)
        self.check('drop')

    def retire_if_unreferenced(self, id_: int) -> None:
        if id_ in self.live and self.refs[id_] == 0:
            self.live.remove(id_)
            if self.pins[id_] != 0:
                self.retiring.add(id_)
        if id_ in self.retiring and self.pins[id_] == 0:
            self.retiring.remove(id_)

    def pin(self, owner: str, id_: int) -> None:
        assert owner not in self.pin_owners and id_ in self.live
        self.pin_owners[owner] = id_
        self.pins[id_] += 1
        self.check('spawn load pin')

    def unpin(self, owner: str) -> None:
        id_ = self.pin_owners.pop(owner)
        self.pins[id_] -= 1
        self.retire_if_unreferenced(id_)
        self.check('load complete or rollback')

    def enqueue(self, idx: int, caller: int, src: int, slot: int) -> None:
        call = self.calls[idx]
        assert call.state == 'Empty'
        id_ = self.spaces[src][slot]
        assert id_ is not None
        call.state, call.caller, call.send_cap = 'Waiting', caller, id_
        self.hook('send_enqueue', id_, 1)
        self.check('Waiting')

    def deliver(self, idx: int, server: int, dest_slot: int, full=False) -> None:
        call = self.calls[idx]
        assert call.state == 'Waiting'
        call.state, call.server = 'Delivered', server
        id_ = call.send_cap
        call.send_cap = None     # IF=0 escrow: count moves, not drop/re-add.
        if not full:
            assert self.spaces[server][dest_slot] is None
            self.spaces[server][dest_slot] = id_
            call.landed = dest_slot
        else:
            self.hook('full_drop', id_, -1)
        self.check('Delivered/full-drop')

    def reply(self, idx: int, server: int, source_slot: int) -> None:
        call = self.calls[idx]
        assert call.state == 'Delivered' and call.server == server
        id_ = self.spaces[server][source_slot]
        assert id_ is not None
        call.reply_cap, call.state = id_, 'Replied'
        self.hook('reply_enqueue', id_, 1)
        self.check('Replied')

    def resume(self, idx: int, dest_slot: int, full=False) -> None:
        call = self.calls[idx]
        assert call.state == 'Replied'
        id_ = call.reply_cap
        call.reply_cap = None    # Count held in escrow until installed/dropped.
        if not full:
            assert self.spaces[call.caller][dest_slot] is None
            self.spaces[call.caller][dest_slot] = id_
        else:
            self.hook('reply_full_drop', id_, -1)
        self.calls[idx] = Call()
        self.check('reply resume')

    def server_death(self, server: int) -> None:
        for call in self.calls:
            if call.state == 'Waiting' or (call.server == server and call.state == 'Delivered'):
                if call.send_cap is not None:
                    self.hook('failed_send_drop', call.send_cap, -1)
                    call.send_cap = None
                call.state = 'Failed'
            # Already-Replied cap belongs to the queued reply, NOT the server.
        self.sweep(server)
        self.check('server death -> Failed, Replied survives')

    def caller_death(self, caller: int) -> None:
        for idx, call in enumerate(self.calls):
            if call.caller == caller and call.state != 'Empty':
                for id_ in (call.send_cap, call.reply_cap):
                    if id_ is not None:
                        self.hook('caller_sweep', id_, -1)
                self.calls[idx] = Call()
        self.sweep(caller)
        self.check('caller death Waiting/Delivered/Replied/Failed')

    def sweep(self, pid: int) -> None:
        for slot, id_ in enumerate(self.spaces[pid]):
            if id_ is not None:
                self.spaces[pid][slot] = None
                self.hook('sweep', id_, -1)
        for id_ in tuple(self.live):
            self.retire_if_unreferenced(id_)

    def revoke(self, id_: int) -> None:
        assert id_ in self.live
        self.live.remove(id_)  # Stale numerics remain in spaces/queues.
        self.refs.pop(id_, None)
        if self.pins[id_] != 0:
            self.retiring.add(id_)
        self.check('all copied refs stale immediately; pins still tracked')


def suite() -> int:
    total = 0
    # Every queue state with caller death; include reply cap land and full drop.
    for state in ('Waiting', 'Delivered', 'Replied', 'Failed'):
        m = Machine()
        m.mint(27, 1, 0)
        m.copy(1, 0, 1, 1)
        m.move(1, 1, 1, 2)
        m.pin('loader', 27)
        m.unpin('loader')
        m.enqueue(0, 1, 1, 0)
        if state != 'Waiting':
            m.deliver(0, 2, 0)
        if state == 'Replied':
            m.reply(0, 2, 0)
            m.server_death(2)
        if state == 'Failed':
            m.server_death(2)
        if state == 'Delivered':
            m.server_death(2)
        m.caller_death(1)
        assert not m.live
        total += m.checks

    # Waiting server death before delivery and caller-first Delivered teardown.
    for delivered in (False, True):
        m = Machine()
        m.mint(27, 1, 0)
        m.enqueue(0, 1, 1, 0)
        if delivered:
            m.deliver(0, 2, 0)
            m.caller_death(1)  # server's already-landed reference survives.
            m.server_death(2)
        else:
            m.server_death(2)  # Waiting transitions to Failed.
            m.caller_death(1)
        assert not m.live
        total += m.checks

    m = Machine()
    m.mint(27, 1, 0)
    m.enqueue(0, 1, 1, 0)
    m.deliver(0, 2, 1, full=True)
    m.caller_death(1)
    m.mint(28, 1, 0)  # Fresh ID on registry slot reuse.
    m.enqueue(1, 1, 1, 0)
    m.deliver(1, 2, 0)
    m.reply(1, 2, 0)
    m.resume(1, 2, full=True)
    m.revoke(28)
    m.sweep(1); m.sweep(2); m.check('stale caps do not revive after reuse')
    total += m.checks

    # Last-cap retirement or explicit revoke while a loader still owns
    # immutable bytes: deny new spawns immediately, free only on unpin.
    for explicit in (False, True):
        m = Machine()
        m.mint(30, 1, 0)
        m.pin('loader', 30)
        if explicit:
            m.revoke(30)
            m.drop(1, 0)
        else:
            m.drop(1, 0)
        assert 30 not in m.live and 30 in m.retiring
        m.unpin('loader')
        assert 30 not in m.retiring
        total += m.checks

    # Successful reply landing; no mutation/extra ref on refused mint/grant.
    m = Machine()
    m.mint(29, 2, 0)
    m.enqueue(0, 1, 2, 0)
    m.deliver(0, 2, 1)
    m.reply(0, 2, 0)
    m.resume(0, 3)
    m.drop(2, 0); m.drop(2, 1); m.drop(1, 3)
    assert not m.live and m.spaces[1][4] is None
    m.check('last ref retired, spawn rollback/refusal leaves empty destination')
    total += m.checks

    # RED: intentionally omit exactly one hook; independent scan must catch it.
    for missing in ('send_enqueue', 'reply_enqueue', 'caller_sweep'):
        m = Machine()
        m.mint(27, 1, 0)
        if missing == 'caller_sweep':
            m.enqueue(0, 1, 1, 0)
            m.skip_hook = missing
            try:
                m.caller_death(1)
            except AssertionError as error:
                assert 'caller death' in str(error)
            else:
                raise AssertionError('RED control did not catch caller_sweep omission')
        else:
            if missing == 'reply_enqueue':
                m.enqueue(0, 1, 1, 0)
                m.deliver(0, 2, 0)
            m.skip_hook = missing
            try:
                (m.reply(0, 2, 0) if missing == 'reply_enqueue' else m.enqueue(0, 1, 1, 0))
            except AssertionError as error:
                assert ('Replied' if missing == 'reply_enqueue' else 'Waiting') in str(error)
            else:
                raise AssertionError(f'RED control did not catch {missing}')
        total += 1
    print(f'host design model PASS: {total} independent stable-boundary checks; '
          '3 omitted-hook RED controls detected; NO production registry/guest proof')
    return 0

if __name__ == '__main__':
    raise SystemExit(suite())
