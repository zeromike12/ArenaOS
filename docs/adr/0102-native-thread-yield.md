# ADR-0102: Native voluntary thread yield

## Status

Accepted for Phase 13.

## Context

The native scheduler already rotates runnable user threads at timer ticks and
blocks threads on ArenaOS notifications and IPC. A CPU-bound native thread
also needs a way to yield its current run voluntarily so cooperative loops can
let another runnable thread proceed without arming a timer or manufacturing a
blocking event.

## Options considered

- Require applications to wait on notifications or timers to give up the CPU.
  This confuses blocking with scheduling and adds unnecessary object use.
- Add a futex-like or scheduler-policy ABI. The current scheduler needs only a
  one-way yield request, not a POSIX wait/wake contract or priority controls.
- Add one no-argument native syscall that places the current runnable thread
  at the scheduler's normal yield point.

## Decision

Syscall 62, `SYS_THREAD_YIELD`, asks the scheduler to run its existing
`yield_now` path. It takes no authority or arguments, returns success, and
does not promise which thread runs next or how soon the caller resumes. It
does not block, create another process, or alter the caller's capabilities.

## Reasoning

This gives native applications an explicit cooperative scheduling point while
leaving timer preemption and blocking semantics unchanged. A yield has no
identity-bearing resource to account and cannot grant authority. The syscall
is separate from any future wait/wake primitive used by Mutex or Condvar.

## Downsides accepted

Yielding is advisory. A single-threaded process sees no useful concurrency,
and a runnable peer may not exist. Programs must not use it as a timing or
synchronization guarantee.

## Future implications

Native synchronization should use a process/thread-aware wait-notify contract
with explicit lost-wakeup rules. Linux futex compatibility, if later needed,
must map onto that native mechanism rather than changing this yield syscall.
