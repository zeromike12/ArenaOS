#ifndef ARENAOS_CLIENT_H
#define ARENAOS_CLIENT_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define ADSK_EVT_NONE      0
#define ADSK_EVT_KEY       1
#define ADSK_EVT_POINTER   2
#define ADSK_EVT_CLOSE     3
#define ADSK_EVT_FOCUS     4
#define ADSK_EVT_CONFIGURE 5
#define ADSK_EVT_CHORD     8
#define ADSK_EVT_WHEEL     9

struct adsk_event {
    uint8_t type;
    union {
        struct {
            uint16_t key;
        } key;
        struct {
            int32_t x;
            int32_t y;
            uint8_t buttons;
        } pointer;
        struct {
            bool focused;
        } focus;
        struct {
            uint16_t width;
            uint16_t height;
        } configure;
        struct {
            uint16_t code;
            uint8_t mods;
        } chord;
        struct {
            int32_t x;
            int32_t y;
            int8_t delta;
        } wheel;
    } data;
};

bool adsk_connect(uint16_t width, uint16_t height, const char *title);
void adsk_disconnect(void);
bool adsk_damage_full(void);
bool adsk_damage_rects(int count, const uint16_t rects[][4]);
bool adsk_poll_event(struct adsk_event *out_event);
void *adsk_get_pixels(void);
uint16_t adsk_get_width(void);
uint16_t adsk_get_height(void);
uint64_t adsk_get_handle(void);
void adsk_sleep_ms(uint32_t ms);

#endif /* ARENAOS_CLIENT_H */
