#include <SDL3/SDL.h>

#define WINDOW_WIDTH  320
#define WINDOW_HEIGHT 240

/* Palette of vibrant colors for the bouncing box */
static const Uint32 palette[] = {
    0x003498DB, /* Dodger Blue */
    0x00E94560, /* Crimson Red */
    0x002ECC71, /* Emerald Green */
    0x00F39C12, /* Orange */
    0x009B59B6, /* Amethyst Purple */
};

static void draw_box(SDL_Surface *surface, int x, int y, int w, int h, Uint32 color) {
    if (!surface || !surface->pixels) return;
    Uint32 *pixels = (Uint32 *)surface->pixels;
    int pitch_pixels = surface->pitch / 4;

    /* Clamp to surface boundaries */
    int x0 = x < 0 ? 0 : x;
    int y0 = y < 0 ? 0 : y;
    int x1 = (x + w) > surface->w ? surface->w : (x + w);
    int y1 = (y + h) > surface->h ? surface->h : (y + h);

    for (int cy = y0; cy < y1; ++cy) {
        for (int cx = x0; cx < x1; ++cx) {
            pixels[cy * pitch_pixels + cx] = color;
        }
    }
}

int main(int argc, char *argv[]) {
    SDL_Log("[main] Entered main");

    /* 1. Initialize SDL video subsystem */
    SDL_Log("[sdl3-app] Initializing SDL3 video subsystem...");
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        SDL_Log("[sdl3-app] Failed to initialize SDL3: %s", SDL_GetError());
        return 1;
    }
    SDL_Log("[sdl3-app] SDL3 video subsystem initialized successfully!");

    /* 2. Create window with title and dimensions */
    SDL_Window *window = SDL_CreateWindow(
        "SDL3 Native ArenaOS Demo",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        0
    );
    if (!window) {
        SDL_Log("[sdl3-app] Failed to create SDL_Window: %s", SDL_GetError());
        SDL_Quit();
        return 1;
    }
    SDL_Log("[sdl3-app] SDL_Window created: %p (id=%u)", (void *)window, SDL_GetWindowID(window));

    /* 3. Acquire window software surface */
    SDL_Surface *surface = SDL_GetWindowSurface(window);
    if (!surface) {
        SDL_Log("[sdl3-app] Failed to acquire window surface: %s", SDL_GetError());
        SDL_DestroyWindow(window);
        SDL_Quit();
        return 1;
    }
    SDL_Log("[sdl3-app] Surface acquired: %dx%d, format=%u, pitch=%d, pixels=%p",
            surface->w, surface->h, surface->format, surface->pitch, surface->pixels);

    /* 4. Interactive animation state */
    int box_x = 40;
    int box_y = 60;
    int box_w = 48;
    int box_h = 48;
    int box_dx = 3;
    int box_dy = 2;
    int color_index = 0;
    int num_colors = sizeof(palette) / sizeof(palette[0]);

    int mouse_x = WINDOW_WIDTH / 2;
    int mouse_y = WINDOW_HEIGHT / 2;
    bool mouse_down = false;

    Uint64 start_time = SDL_GetTicks();
    Uint64 total_render_ns = 0;
    Uint64 total_present_ns = 0;
    int frame_count = 0;
    const int max_frames = 60;
    bool running = true;

    SDL_Log("[sdl3-app] Entering interactive frame loop (running for %d frames)...", max_frames);

    while (running && frame_count < max_frames) {
        /* Poll and process incoming SDL3 events */
        SDL_Event event;
        while (SDL_PollEvent(&event)) {
            switch (event.type) {
                case SDL_EVENT_QUIT:
                    SDL_Log("[sdl3-app] Event: SDL_EVENT_QUIT received");
                    running = false;
                    break;

                case SDL_EVENT_WINDOW_CLOSE_REQUESTED:
                    SDL_Log("[sdl3-app] Event: SDL_EVENT_WINDOW_CLOSE_REQUESTED received");
                    running = false;
                    break;

                case SDL_EVENT_MOUSE_MOTION:
                    mouse_x = (int)event.motion.x;
                    mouse_y = (int)event.motion.y;
                    break;

                case SDL_EVENT_MOUSE_BUTTON_DOWN:
                    mouse_down = true;
                    color_index = (color_index + 1) % num_colors;
                    SDL_Log("[sdl3-app] Event: Mouse button down at (%d, %d)", mouse_x, mouse_y);
                    break;

                case SDL_EVENT_MOUSE_BUTTON_UP:
                    mouse_down = false;
                    break;

                case SDL_EVENT_KEY_DOWN:
                    if (event.key.scancode == SDL_SCANCODE_SPACE) {
                        color_index = (color_index + 1) % num_colors;
                        SDL_Log("[sdl3-app] Event: Spacebar pressed -> cycling color");
                    } else if (event.key.scancode == SDL_SCANCODE_ESCAPE) {
                        SDL_Log("[sdl3-app] Event: Escape pressed -> exiting");
                        running = false;
                    }
                    break;

                default:
                    break;
            }
        }

        Uint64 t0 = SDL_GetTicksNS();

        /* 1. Clear background to dark slate (0x001A1A2E) */
        draw_box(surface, 0, 0, WINDOW_WIDTH, WINDOW_HEIGHT, 0x001A1A2E);

        /* 2. Draw sub-header banner inside window below 28px desktop chrome */
        draw_box(surface, 0, 28, WINDOW_WIDTH, 20, 0x0016213E);
        draw_box(surface, 8, 32, 12, 12, 0x00E94560);
        draw_box(surface, 26, 32, 12, 12, 0x000F3460);
        draw_box(surface, 44, 32, 12, 12, 0x0053354A);

        /* 3. Update bouncing box */
        box_x += box_dx;
        box_y += box_dy;
        if (box_x <= 0 || box_x + box_w >= WINDOW_WIDTH) {
            box_dx = -box_dx;
            color_index = (color_index + 1) % num_colors;
        }
        if (box_y <= 48 || box_y + box_h >= WINDOW_HEIGHT - 6) {
            box_dy = -box_dy;
            color_index = (color_index + 1) % num_colors;
        }

        /* Draw bouncing box with inner highlight */
        Uint32 box_color = palette[color_index];
        draw_box(surface, box_x, box_y, box_w, box_h, box_color);
        draw_box(surface, box_x + 4, box_y + 4, box_w - 8, box_h - 8, 0x00FFFFFF);
        draw_box(surface, box_x + 8, box_y + 8, box_w - 16, box_h - 16, box_color);

        /* 4. Draw interactive mouse indicator */
        int cur_size = mouse_down ? 16 : 8;
        int cur_x = mouse_x - cur_size / 2;
        int cur_y = mouse_y - cur_size / 2;
        draw_box(surface, cur_x, cur_y, cur_size, cur_size, mouse_down ? 0x00FFD700 : 0x0000FFFF);

        /* 5. Draw frame progress bar at bottom */
        int progress_w = ((frame_count + 1) * WINDOW_WIDTH) / max_frames;
        draw_box(surface, 0, WINDOW_HEIGHT - 4, progress_w, 4, 0x0000FF88);

        Uint64 t1 = SDL_GetTicksNS();
        total_render_ns += (t1 - t0);

        /* 6. Present to Desktop broker via ADSK-v1 damage */
        SDL_UpdateWindowSurface(window);

        Uint64 t2 = SDL_GetTicksNS();
        total_present_ns += (t2 - t1);

        if (frame_count == 0 || frame_count == 25 || frame_count == 59) {
            SDL_Log("[sdl3-app] frame rendered; damage published; frame=%d (box at %d,%d)",
                    frame_count, box_x, box_y);
        }

        /* 7. Frame pacing */
        SDL_Delay(16);
        frame_count++;
    }

    Uint64 elapsed_ms = SDL_GetTicks() - start_time;
    SDL_Log("[sdl3-app] Animation completed: %d frames in %lu ms (~%lu FPS)",
            frame_count, (unsigned long)elapsed_ms, (unsigned long)((frame_count * 1000UL) / (elapsed_ms ? elapsed_ms : 1)));
    SDL_Log("[sdl3-app] Timing breakdown: render=%lu us, present=%lu us",
            (unsigned long)(total_render_ns / 1000ULL), (unsigned long)(total_present_ns / 1000ULL));

    /* Clean shutdown via public SDL3 APIs */
    SDL_Log("[sdl3-app] Shutting down SDL3 window and video subsystem...");
    SDL_DestroyWindow(window);
    SDL_Quit();
    SDL_Log("[sdl3-app] Clean termination with exit code 0");

    return 0;
}
