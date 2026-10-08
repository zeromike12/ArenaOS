/*
  Authentic Upstream SDL 3.2.0 Demonstration Application for ArenaOS
  Exercising genuine upstream SDL3 interfaces linked against libSDL3_upstream.a
*/

#include <SDL3/SDL.h>

#define WINDOW_WIDTH  320
#define WINDOW_HEIGHT 240

/* Palette of vibrant colors for animation */
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
    SDL_Log("[upstream-demo] Starting Genuine Upstream SDL 3.2.0 Demo on ArenaOS...");

    /* 1. Initialize SDL video subsystem via genuine upstream SDL_Init */
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        SDL_Log("[upstream-demo] Failed to initialize SDL video: %s", SDL_GetError());
        return 1;
    }
    SDL_Log("[upstream-demo] Upstream SDL video subsystem initialized successfully!");

    /* 2. Create window via genuine upstream SDL_CreateWindow */
    SDL_Window *window = SDL_CreateWindow(
        "Upstream SDL 3.2.0 Demo",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        0
    );
    if (!window) {
        SDL_Log("[upstream-demo] Failed to create SDL_Window: %s", SDL_GetError());
        SDL_Quit();
        return 1;
    }
    SDL_Log("[upstream-demo] Upstream SDL_Window created: %p (id=%u)", (void *)window, SDL_GetWindowID(window));

    /* 3. Acquire window surface via genuine upstream SDL_GetWindowSurface */
    SDL_Surface *surface = SDL_GetWindowSurface(window);
    if (!surface) {
        SDL_Log("[upstream-demo] Failed to acquire window surface: %s", SDL_GetError());
        SDL_DestroyWindow(window);
        SDL_Quit();
        return 1;
    }
    SDL_Log("[upstream-demo] Surface acquired: %dx%d, format=%u, pitch=%d, pixels=%p",
            surface->w, surface->h, surface->format, surface->pitch, surface->pixels);

    /* 4. Frame loop */
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
    int frame_count = 0;
    const int max_frames = 60;
    bool running = true;

    SDL_Log("[upstream-demo] Entering interactive frame loop (%d frames)...", max_frames);

    while (running && frame_count < max_frames) {
        SDL_Event event;
        while (SDL_PollEvent(&event)) {
            switch (event.type) {
                case SDL_EVENT_QUIT:
                case SDL_EVENT_WINDOW_CLOSE_REQUESTED:
                    running = false;
                    break;
                case SDL_EVENT_MOUSE_MOTION:
                    mouse_x = (int)event.motion.x;
                    mouse_y = (int)event.motion.y;
                    break;
                case SDL_EVENT_MOUSE_BUTTON_DOWN:
                    mouse_down = true;
                    color_index = (color_index + 1) % num_colors;
                    break;
                case SDL_EVENT_MOUSE_BUTTON_UP:
                    mouse_down = false;
                    break;
                case SDL_EVENT_KEY_DOWN:
                    if (event.key.scancode == SDL_SCANCODE_SPACE) {
                        color_index = (color_index + 1) % num_colors;
                    } else if (event.key.scancode == SDL_SCANCODE_ESCAPE) {
                        running = false;
                    }
                    break;
                default:
                    break;
            }
        }

        /* Draw background */
        draw_box(surface, 0, 0, WINDOW_WIDTH, WINDOW_HEIGHT, 0x001A1A2E);

        /* Draw banner */
        draw_box(surface, 0, 28, WINDOW_WIDTH, 20, 0x0016213E);
        draw_box(surface, 8, 32, 12, 12, 0x00E94560);
        draw_box(surface, 26, 32, 12, 12, 0x000F3460);
        draw_box(surface, 44, 32, 12, 12, 0x0053354A);

        /* Update box */
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

        /* Draw bouncing box */
        Uint32 box_color = palette[color_index];
        draw_box(surface, box_x, box_y, box_w, box_h, box_color);
        draw_box(surface, box_x + 4, box_y + 4, box_w - 8, box_h - 8, 0x00FFFFFF);
        draw_box(surface, box_x + 8, box_y + 8, box_w - 16, box_h - 16, box_color);

        /* Draw mouse indicator */
        int cur_size = mouse_down ? 16 : 8;
        draw_box(surface, mouse_x - cur_size / 2, mouse_y - cur_size / 2, cur_size, cur_size, mouse_down ? 0x00FFD700 : 0x0000FFFF);

        /* Present via genuine upstream UpdateWindowSurface */
        SDL_UpdateWindowSurface(window);

        if (frame_count == 0 || frame_count == 25 || frame_count == 59) {
            SDL_Log("[upstream-demo] frame rendered; damage published; frame=%d", frame_count);
        }

        SDL_Delay(16);
        frame_count++;
    }

    Uint64 elapsed_ms = SDL_GetTicks() - start_time;
    SDL_Log("[upstream-demo] Completed %d frames in %lu ms", frame_count, (unsigned long)elapsed_ms);

    /* 5. Clean teardown via genuine upstream SDL_DestroyWindow and SDL_Quit */
    SDL_Log("[upstream-demo] Shutting down upstream SDL...");
    SDL_DestroyWindow(window);
    SDL_Quit();
    SDL_Log("[upstream-demo] Clean exit 0");

    return 0;
}
