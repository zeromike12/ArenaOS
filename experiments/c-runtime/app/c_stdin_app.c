/*
 * Phase 16 live-stdin demonstration app (EXPERIMENTAL).
 *
 * Launched WINDOWED with granted standard streams. The desktop routes
 * keyboard bytes for the focused window into this process's stdin ring
 * (existing ADR-0104 plumbing); Ctrl-D closes the desktop's stdin writer
 * (EOF). The app echoes each received line to stdout with its length and
 * a running line count, answers "ping" with "pong", and exits 73 on EOF
 * after printing a summary. Exit 58 = stdio could not attach (no grant);
 * exit 74 = stream error other than EOF.
 *
 * The app never touches graphics: the desktop owns the window frame and
 * title; this process simply never maps a surface (B-GFX-1: no service
 * guessing). If the startup record has no unambiguous wait notification,
 * blocking reads refuse with ARENA_E_NO_WAITER and the app falls back to
 * a yield-poll, saying so on stdout (mode=poll) instead of pretending.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <threads.h>

#include "arena/rt.h"

#ifndef APP_NAME /* guest_stdin.py builds two instances with distinct tags */
#define APP_NAME "[c-stdin]"
#endif
#define EXIT_EOF_OK 73
#define EXIT_STREAM_ERROR 74
#define EXIT_NO_STREAMS 58

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    if (arena_stdio_init() != 0) {
        return EXIT_NO_STREAMS;
    }
    const arena_startup_t *s = arena_startup();
    int can_block = s->present && s->notification != ARENA_ARST_NONE;
    printf(APP_NAME " ready mode=%s\n", can_block ? "block" : "poll");
    fflush(stdout);

    char line[160];
    size_t used = 0;
    unsigned lines = 0, bytes = 0;
    for (;;) {
        char ch;
        long n = arena_stream_read(0, &ch, 1);
        if (n == ARENA_E_NO_WAITER) {
            /* Poll mode: non-blocking peek + cooperative yield. */
            long m = arena_stream_read_some(0, &ch, 1);
            if (m == ARENA_E_WOULD_BLOCK) {
                thrd_yield();
                continue;
            }
            n = m;
        }
        if (n == 0) { /* EOF: desktop writer closed and ring drained */
            break;
        }
        if (n < 0) {
            printf(APP_NAME " stream error %ld\n", n);
            fflush(stdout);
            return EXIT_STREAM_ERROR;
        }
        bytes++;
        if (ch == '\r') {
            ch = '\n';
        }
        if (ch == '\n') {
            line[used] = '\0';
            lines++;
            if (strcmp(line, "ping") == 0) {
                printf(APP_NAME " pong line=%u\n", lines);
            } else {
                printf(APP_NAME " echo len=%u text=%s\n", (unsigned)used, line);
            }
            fflush(stdout);
            used = 0;
        } else if (ch == '\b' || ch == 0x7F) {
            if (used > 0) {
                used--;
            }
        } else if (used + 1 < sizeof line) {
            line[used++] = ch;
        }
    }
    printf(APP_NAME " EOF lines=%u bytes=%u\n", lines, bytes);
    fflush(stdout);
    return EXIT_EOF_OK;
}
