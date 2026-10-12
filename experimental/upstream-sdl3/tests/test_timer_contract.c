/*
  test_timer_contract.c: Unit Test for ArenaOS Timer and Notification Rights Contract
  
  Verifies:
  1. Desktop grants Slot 3 (CLOCK_SLOT) as Notification with READ | WRITE rights (ADR-0071, desktop.rs:3581).
  2. SYS_TIMER_ARM (syscall 27) takes (slot, badge, relative_delay_us) and requires Notification WRITE.
  3. SYS_WAIT (syscall 11) takes (slot) and requires Notification READ.
  4. Waiting on a WRITE-only notification returns STATUS_BAD_ARG.
  5. SYS_TIMER_CANCEL (syscall 28) takes timer_id, NOT capability slot index.
  6. Client sleep adapter logic:
     - On normal delivery (badge & timer_badge != 0), timer is already disarmed and NOT cancelled.
     - On early wake by another event (badge & timer_badge == 0), the active timer is safely cancelled.

  NOTE: This host test verifies the capability rights and syscall contract semantics.
  It is a contract test and does not substitute for real QEMU guest execution.
*/

#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>
#include <assert.h>

#define STATUS_OK 0
#define STATUS_BAD_ARG -1
#define STATUS_BUSY -2

#define SYS_WAIT          11
#define SYS_TIMER_ARM     27
#define SYS_TIMER_CANCEL  28

#define CAP_KIND_NOTIFICATION 3
#define CAP_KIND_SHARED_REGION 7
#define CAP_KIND_BADGED_ENDPOINT 12

#define RIGHT_READ 1
#define RIGHT_WRITE 2
#define RIGHT_COPY 4

#define CLOCK_SLOT 3

typedef struct {
    uint8_t kind;
    uint32_t rights;
    uint32_t pending_badge;
} MockCap;

static MockCap s_caps[8];

typedef struct {
    bool active;
    uint32_t id;
    uint32_t slot;
    uint32_t badge;
    uint64_t delay_us;
} MockTimer;

static MockTimer s_timers[4];
static uint32_t s_next_timer_id = 100; /* Distinct from cap slot numbers */
static int s_cancel_calls = 0;

static int64_t mock_sys_timer_arm(uint64_t slot, uint64_t badge, uint64_t delay_us) {
    if (slot >= 8) return STATUS_BAD_ARG;
    MockCap *c = &s_caps[slot];
    if (c->kind != CAP_KIND_NOTIFICATION) return STATUS_BAD_ARG;
    if ((c->rights & RIGHT_WRITE) == 0) return STATUS_BAD_ARG; /* Needs WRITE */

    for (int i = 0; i < 4; i++) {
        if (!s_timers[i].active) {
            uint32_t tid = s_next_timer_id++;
            s_timers[i].active = true;
            s_timers[i].id = tid;
            s_timers[i].slot = (uint32_t)slot;
            s_timers[i].badge = (uint32_t)badge;
            s_timers[i].delay_us = delay_us;
            return (int64_t)tid;
        }
    }
    return STATUS_BUSY;
}

static int64_t mock_sys_timer_cancel(uint64_t timer_id) {
    s_cancel_calls++;
    if (timer_id == 0) return STATUS_BAD_ARG;
    for (int i = 0; i < 4; i++) {
        if (s_timers[i].active && s_timers[i].id == (uint32_t)timer_id) {
            s_timers[i].active = false;
            return STATUS_OK;
        }
    }
    return STATUS_BAD_ARG; /* Already expired, disarmed, or nonexistent */
}

static int64_t mock_sys_wait(uint64_t slot) {
    if (slot >= 8) return STATUS_BAD_ARG;
    MockCap *c = &s_caps[slot];
    if (c->kind != CAP_KIND_NOTIFICATION) return STATUS_BAD_ARG;
    if ((c->rights & RIGHT_READ) == 0) {
        /* Waiting on a notification requires READ authority! */
        return STATUS_BAD_ARG;
    }

    /* If no pending badge already waiting (e.g. external event), deliver active timer */
    if (c->pending_badge == 0) {
        for (int i = 0; i < 4; i++) {
            if (s_timers[i].active && s_timers[i].slot == slot) {
                c->pending_badge |= s_timers[i].badge;
                s_timers[i].active = false; /* Timer delivered and disarmed by kernel */
            }
        }
    }

    uint32_t badge = c->pending_badge;
    c->pending_badge = 0;
    return (int64_t)badge;
}

/* Simulation of arenaos_client.c adsk_sleep_us adapter */
static void client_adapter_sleep_us(uint32_t us, uint32_t simulated_external_event) {
    uint32_t delay = (us == 0) ? 1 : us;
    uint32_t timer_badge = 1;
    int64_t timer_id = mock_sys_timer_arm(CLOCK_SLOT, timer_badge, delay);
    assert(timer_id >= 0);

    /* If an external event woke the process before timer expiration */
    if (simulated_external_event != 0) {
        s_caps[CLOCK_SLOT].pending_badge |= simulated_external_event;
    }

    int64_t w = mock_sys_wait(CLOCK_SLOT);
    if (w >= 0 && (w & timer_badge) == 0) {
        /* Woke by external event, cancel still-pending timer */
        mock_sys_timer_cancel((uint64_t)timer_id);
    }
}

int main(void) {
    printf("=== Testing Timer and Notification Rights Contract ===\n");

    /* Setup mock process capability table matching Desktop launch grants:
       Slot 1: Desktop BadgedEndpoint (WRITE)
       Slot 2: Window Surface (READ | WRITE | COPY)
       Slot 3: Desktop Clock Notification (READ | WRITE) [desktop.rs:3581]
       Slot 4: Restricted WRITE-only Notification (hypothetical negative control)
    */
    s_caps[1].kind = CAP_KIND_BADGED_ENDPOINT; s_caps[1].rights = RIGHT_WRITE;
    s_caps[2].kind = CAP_KIND_SHARED_REGION;   s_caps[2].rights = RIGHT_READ | RIGHT_WRITE | RIGHT_COPY;
    s_caps[3].kind = CAP_KIND_NOTIFICATION;    s_caps[3].rights = RIGHT_READ | RIGHT_WRITE;
    s_caps[4].kind = CAP_KIND_NOTIFICATION;    s_caps[4].rights = RIGHT_WRITE;

    /* Test 1: SYS_TIMER_ARM on non-notification cap fails */
    printf("[test_timer] Test 1: SYS_TIMER_ARM on non-notification rejected...\n");
    assert(mock_sys_timer_arm(1, 1, 1000) == STATUS_BAD_ARG);
    assert(mock_sys_timer_arm(2, 1, 1000) == STATUS_BAD_ARG);
    printf("  [PASS] Arming timer on non-notification returned STATUS_BAD_ARG.\n");

    /* Test 2: SYS_TIMER_ARM on notification with WRITE succeeds */
    printf("[test_timer] Test 2: SYS_TIMER_ARM on WRITE notification succeeds...\n");
    int64_t t3 = mock_sys_timer_arm(3, 1, 1000);
    assert(t3 >= 100);
    int64_t t4 = mock_sys_timer_arm(4, 2, 2000);
    assert(t4 >= 100);
    assert(mock_sys_timer_cancel(t3) == STATUS_OK);
    assert(mock_sys_timer_cancel(t4) == STATUS_OK);
    printf("  [PASS] Arming timer with WRITE rights returned valid positive timer_id.\n");

    /* Test 3: SYS_WAIT on WRITE-only notification fails closed */
    printf("[test_timer] Test 3: SYS_WAIT on WRITE-only notification rejected...\n");
    int64_t tid = mock_sys_timer_arm(4, 1, 5000);
    assert(tid >= 100);
    int64_t w_result = mock_sys_wait(4); /* Slot 4 has NO READ rights! */
    assert(w_result == STATUS_BAD_ARG);
    assert(mock_sys_timer_cancel(tid) == STATUS_OK);
    printf("  [PASS] SYS_WAIT on WRITE-only notification correctly refused with STATUS_BAD_ARG.\n");

    /* Test 4: SYS_WAIT on Slot 3 (READ|WRITE) succeeds and delivers timer badge */
    printf("[test_timer] Test 4: SYS_WAIT on READ|WRITE Slot 3 succeeds...\n");
    tid = mock_sys_timer_arm(3, 0x123, 1000);
    assert(tid >= 100);
    w_result = mock_sys_wait(3); /* Slot 3 has READ authority */
    assert(w_result == 0x123);
    printf("  [PASS] SYS_WAIT returned delivered badge.\n");

    /* Test 5: Timer cancellation contract: cancel takes timer_id, NOT slot */
    printf("[test_timer] Test 5: SYS_TIMER_CANCEL takes timer_id, not cap slot...\n");
    tid = mock_sys_timer_arm(3, 1, 1000);
    assert(tid >= 100);
    assert(mock_sys_timer_cancel(3) == STATUS_BAD_ARG);
    assert(mock_sys_timer_cancel(tid) == STATUS_OK);
    assert(mock_sys_timer_cancel(tid) == STATUS_BAD_ARG);
    printf("  [PASS] Cancellation contract verified.\n");

    /* Test 6: Client sleep adapter verification:
       Case A: Normal timer expiration -> timer delivered, NOT cancelled.
       Case B: Interrupted by external badge -> timer still active, cancelled. */
    printf("[test_timer] Test 6: Client adapter sleep behavior...\n");
    s_cancel_calls = 0;
    client_adapter_sleep_us(1000, 0); /* Normal timer wake */
    assert(s_cancel_calls == 0); /* Must NOT call cancel when timer expired */

    s_cancel_calls = 0;
    client_adapter_sleep_us(1000, 0x02); /* Woken early by external badge 0x02 */
    assert(s_cancel_calls == 1); /* Must cancel the still-pending timer */
    printf("  [PASS] Normal delivery and early cancellation verified.\n");

    printf("\nALL TIMER CONTRACT TESTS PASSED!\n");
    return 0;
}
