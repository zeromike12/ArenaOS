#include "SDL_arenaosvideo.h"

extern void SDL_SendWindowEvent(SDL_Window *window, SDL_EventType windowevent, int data1, int data2);
extern void SDL_SendKeyboardKey(Uint64 timestamp, SDL_KeyboardID keyboardID, int rawcode, SDL_Scancode scancode, bool down);
extern void SDL_SendMouseMotion(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, bool relative, float x, float y);
extern void SDL_SendMouseButton(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, Uint8 button, bool down);
extern void SDL_SendMouseWheel(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, float x, float y, SDL_MouseWheelDirection direction);

static uint8_t s_prev_mouse_buttons = 0;

static SDL_Scancode translate_key_to_scancode(uint16_t key) {
    if (key >= 'a' && key <= 'z') {
        return (SDL_Scancode)(SDL_SCANCODE_A + (key - 'a'));
    } else if (key >= 'A' && key <= 'Z') {
        return (SDL_Scancode)(SDL_SCANCODE_A + (key - 'A'));
    } else if (key >= '1' && key <= '9') {
        return (SDL_Scancode)(SDL_SCANCODE_1 + (key - '1'));
    } else if (key == '0') {
        return SDL_SCANCODE_0;
    } else if (key == 27) {
        return SDL_SCANCODE_ESCAPE;
    } else if (key == 13 || key == 10) {
        return SDL_SCANCODE_RETURN;
    } else if (key == 8) {
        return SDL_SCANCODE_BACKSPACE;
    } else if (key == 32) {
        return SDL_SCANCODE_SPACE;
    } else if (key == 9) {
        return SDL_SCANCODE_TAB;
    }
    return SDL_SCANCODE_UNKNOWN;
}

void ARENAOS_PumpEvents(SDL_VideoDevice *_this) {
    if (!_this) return;

    SDL_Window *window = _this->windows;
    struct adsk_event ev;

    while (adsk_poll_event(&ev)) {
        switch (ev.type) {
            case ADSK_EVT_KEY: {
                SDL_Scancode scancode = translate_key_to_scancode(ev.data.key.key);
                SDL_SendKeyboardKey(0, 0, (int)ev.data.key.key, scancode, true);
                SDL_SendKeyboardKey(0, 0, (int)ev.data.key.key, scancode, false);
                break;
            }
            case ADSK_EVT_POINTER: {
                if (window) {
                    SDL_SendMouseMotion(0, window, 0, false,
                                         (float)ev.data.pointer.x, (float)ev.data.pointer.y);

                    uint8_t curr = ev.data.pointer.buttons;
                    uint8_t diff = curr ^ s_prev_mouse_buttons;
                    if (diff & 1) {
                        SDL_SendMouseButton(0, window, 0, SDL_BUTTON_LEFT, (curr & 1) != 0);
                    }
                    if (diff & 2) {
                        SDL_SendMouseButton(0, window, 0, SDL_BUTTON_RIGHT, (curr & 2) != 0);
                    }
                    if (diff & 4) {
                        SDL_SendMouseButton(0, window, 0, SDL_BUTTON_MIDDLE, (curr & 4) != 0);
                    }
                    s_prev_mouse_buttons = curr;
                }
                break;
            }
            case ADSK_EVT_WHEEL: {
                if (window) {
                    SDL_SendMouseWheel(0, window, 0, 0.0f, (float)ev.data.wheel.delta, SDL_MOUSEWHEEL_NORMAL);
                }
                break;
            }
            case ADSK_EVT_CLOSE: {
                if (window) {
                    SDL_SendWindowEvent(window, SDL_EVENT_WINDOW_CLOSE_REQUESTED, 0, 0);
                }
                break;
            }
            case ADSK_EVT_FOCUS: {
                if (window) {
                    SDL_SendWindowEvent(window, ev.data.focus.focused ? SDL_EVENT_WINDOW_FOCUS_GAINED : SDL_EVENT_WINDOW_FOCUS_LOST, 0, 0);
                }
                break;
            }
            case ADSK_EVT_CONFIGURE: {
                if (window) {
                    SDL_SendWindowEvent(window, SDL_EVENT_WINDOW_RESIZED, (int)ev.data.configure.width, (int)ev.data.configure.height);
                }
                break;
            }
            default:
                break;
        }
    }
}
