#include <SDL3/SDL.h>
#include <stdio.h>
#include <assert.h>
#include <time.h>

/* Host test stubs for video device and timer sys backends */
void *SDL_GetVideoDevice(void) { return NULL; }
Uint64 SDL_SYS_GetTicksNS(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (Uint64)ts.tv_sec * 1000000000ULL + (Uint64)ts.tv_nsec;
}
Uint64 SDL_SYS_GetTicks(void) {
    return SDL_SYS_GetTicksNS() / 1000000ULL;
}
void SDL_SYS_DelayNS(Uint64 ns) {
    struct timespec ts;
    ts.tv_sec = ns / 1000000000ULL;
    ts.tv_nsec = ns % 1000000000ULL;
    nanosleep(&ts, NULL);
}

extern void SDL_SendKeyboardKey(Uint64 timestamp, SDL_KeyboardID keyboardID, int rawcode, SDL_Scancode scancode, bool down);

extern void SDL_SendMouseMotion(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, bool relative, float x, float y);
extern void SDL_SendMouseButton(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, Uint8 button, bool down);
extern void SDL_SendMouseWheel(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, float x, float y, SDL_MouseWheelDirection direction);
extern void SDL_SendWindowEvent(SDL_Window *window, SDL_EventType windowevent, int data1, int data2);

int main(void) {
    printf("=== Running Event Translation Unit Tests ===\n");

    /* 1. Test Keyboard Event Translation */
    SDL_SendKeyboardKey(1000, 0, 'a', SDL_SCANCODE_A, true);
    SDL_Event ev;
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_KEY_DOWN);
    assert(ev.key.key == 'a');
    assert(ev.key.scancode == SDL_SCANCODE_A);
    assert(ev.key.down == true);

    SDL_SendKeyboardKey(1050, 0, 'a', SDL_SCANCODE_A, false);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_KEY_UP);
    assert(ev.key.key == 'a');
    assert(ev.key.down == false);
    printf("[PASS] Keyboard event translation verified\n");

    /* 2. Test Mouse Motion Translation */
    SDL_SendMouseMotion(2000, NULL, 0, false, 150.0f, 220.0f);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_MOUSE_MOTION);
    assert(ev.motion.x == 150.0f);
    assert(ev.motion.y == 220.0f);
    printf("[PASS] Mouse motion translation verified\n");

    /* 3. Test Mouse Button Translation */
    SDL_SendMouseButton(2100, NULL, 0, SDL_BUTTON_LEFT, true);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_MOUSE_BUTTON_DOWN);
    assert(ev.button.button == SDL_BUTTON_LEFT);
    assert(ev.button.down == true);

    SDL_SendMouseButton(2200, NULL, 0, SDL_BUTTON_LEFT, false);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_MOUSE_BUTTON_UP);
    assert(ev.button.button == SDL_BUTTON_LEFT);
    assert(ev.button.down == false);
    printf("[PASS] Mouse button translation verified\n");

    /* 4. Test Mouse Wheel Translation */
    SDL_SendMouseWheel(2300, NULL, 0, 0.0f, -3.0f, SDL_MOUSEWHEEL_NORMAL);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_MOUSE_WHEEL);
    assert(ev.wheel.y == -3.0f);
    printf("[PASS] Mouse wheel translation verified\n");

    /* 5. Test Window Event Translation */
    SDL_SendWindowEvent(NULL, SDL_EVENT_WINDOW_CLOSE_REQUESTED, 0, 0);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_WINDOW_CLOSE_REQUESTED);

    SDL_SendWindowEvent(NULL, SDL_EVENT_WINDOW_FOCUS_GAINED, 0, 0);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_WINDOW_FOCUS_GAINED);

    SDL_SendWindowEvent(NULL, SDL_EVENT_WINDOW_RESIZED, 640, 480);
    assert(SDL_PollEvent(&ev) == true);
    assert(ev.type == SDL_EVENT_WINDOW_RESIZED);
    assert(ev.window.data1 == 640);
    assert(ev.window.data2 == 480);
    printf("[PASS] Window event translation verified\n");

    /* Verify queue is now empty */
    assert(SDL_PollEvent(&ev) == false);
    printf("[PASS] Queue drainage verified\n");

    printf("=== All Event Translation Tests PASSED ===\n");
    return 0;
}
