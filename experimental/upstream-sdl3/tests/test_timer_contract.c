/*
  test_timer_contract.c: Unit Test for ArenaOS Timer and Notification Rights Contract
  Validates SYS_TIMER_ARM (requires WRITE), SYS_WAIT (requires READ),
  timer-ID cancellation contract, and rejection of wait on WRITE-only notifications.
*/

#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>
#include <assert.h>

#define STATUS_OK 0
#define STATUS_BAD_ARG -1
#define STATUS_BUSY -2

#define CAP_KIND_NOTIFICATION 3
#define CAP_KIND_SHARED_REGION 7
#define CAP_KIND_BADGED_ENDPOINT 12

#define RIGHT_READ 1
#define RIGHT_WRITE 2

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
static uint32_t s_next_timer_id = 100; /* Start IDs above cap slot numbers */

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

    /* Simulate timer badge delivery */
    for (int i = 0; i < 4; i++) {
        if (s_timers[i].active && s_timers[i].slot == slot) {
            c->pending_badge |= s_timers[i].badge;
            s_timers[i].active = false; /* Timer delivered, now disarmed */
        }
    }

    uint32_t badge = c->pending_badge;
    c->pending_badge = 0;
    return (int64_t)badge;
}

int main(void) {
    printf("=== Testing Timer and Notification Rights Contract ===\n");

    /* Setup mock process capability table:
       Slot 1: Desktop BadgedEndpoint (WRITE)
       Slot 2: Window Surface (READ | WRITE)
       Slot 3: Standard Desktop Clock Notification (READ | WRITE)
       Slot 4: Restricted WRITE-only Notification (e.g. StreamWake)
    */
    s_caps[1].kind = CAP_KIND_BADGED_ENDPOINT; s_caps[1].rights = RIGHT_WRITE;
    s_caps[2].kind = CAP_KIND_SHARED_REGION;   s_caps[2].rights = RIGHT_READ | RIGHT_WRITE;
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
    /* Safe client cleans up armed timer */
    assert(mock_sys_timer_cancel(tid) == STATUS_OK);
    printf("  [PASS] SYS_WAIT on WRITE-only notification correctly refused with STATUS_BAD_ARG.\n");

    /* Test 4: SYS_WAIT on READ|WRITE notification succeeds */
    printf("[test_timer] Test 4: SYS_WAIT on READ|WRITE notification succeeds...\n");
    tid = mock_sys_timer_arm(3, 0x123, 1000);
    assert(tid >= 100);
    w_result = mock_sys_wait(3); /* Slot 3 has READ authority */
    assert(w_result == 0x123);
    printf("  [PASS] SYS_WAIT returned delivered badge.\n");

    /* Test 5: Timer cancellation contract: cancel takes timer_id, NOT slot */
    printf("[test_timer] Test 5: SYS_TIMER_CANCEL takes timer_id, not cap slot...\n");
    tid = mock_sys_timer_arm(3, 1, 1000);
    assert(tid >= 100);
    /* Attempting to cancel using slot number (3) instead of timer_id fails */
    assert(mock_sys_timer_cancel(3) == STATUS_BAD_ARG);
    assert(mock_sys_timer_cancel(tid) == STATUS_OK);
    /* Cancelling already disarmed timer fails */
    assert(mock_sys_timer_cancel(tid) == STATUS_BAD_ARG);
    printf("  [PASS] Cancellation contract verified.\n");

    printf("\nALL 5 TIMER CONTRACT TESTS PASSED!\n");
    return 0;
}
