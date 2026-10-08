# Makefile for Experimental SDL3 ArenaOS Platform Backend & Demo App

CC ?= gcc
AR ?= ar

CFLAGS_COMMON = -Iinclude -Isrc -Os -Wall -Wextra -Wno-unused-parameter
CFLAGS_ARENAOS = $(CFLAGS_COMMON) -ffreestanding -fno-builtin -nostdlib -fno-pie -no-pie
LDFLAGS_ARENAOS = -Os -ffreestanding -nostdlib -static -fno-pie -no-pie -Wl,-e,_start -Wl,-Ttext=0x200000 -Wl,--build-id=none

SRCS = \
	src/SDL.c \
	src/SDL_error.c \
	src/SDL_stdlib.c \
	src/SDL_timer.c \
	src/SDL_rect.c \
	src/SDL_pixels.c \
	src/SDL_surface.c \
	src/SDL_events.c \
	src/SDL_keyboard.c \
	src/SDL_mouse.c \
	src/SDL_windowevents.c \
	src/SDL_log.c \
	src/video/SDL_video.c \
	src/video/arenaos/SDL_arenaosvideo.c \
	src/video/arenaos/SDL_arenaosframebuffer.c \
	src/video/arenaos/SDL_arenaosevents.c \
	src/video/arenaos/SDL_arenaostime.c \
	src/platform/arenaos/arenaos_entry.c \
	src/platform/arenaos/arenaos_memory.c \
	src/platform/arenaos/arenaos_client.c

OBJS = $(patsubst src/%.c, build/%.o, $(SRCS))

LIB = build/libSDL3.a
APP_BIN = ../sdl3-app/build/sdl3_app
APP_BUNDLE = ../sdl3-app/SDL3App.apb1

all: $(LIB) $(APP_BIN) $(APP_BUNDLE)

build/%.o: src/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS_ARENAOS) -c $< -o $@

$(LIB): $(OBJS)
	@mkdir -p build
	$(AR) rcs $@ $(OBJS)

$(APP_BIN): ../sdl3-app/src/main.c $(LIB)
	@mkdir -p ../sdl3-app/build
	$(CC) $(CFLAGS_ARENAOS) -c ../sdl3-app/src/main.c -o ../sdl3-app/build/main.o
	$(CC) $(LDFLAGS_ARENAOS) ../sdl3-app/build/main.o -Lbuild -lSDL3 /usr/lib/gcc/x86_64-linux-gnu/12/libgcc.a -o $@

$(APP_BUNDLE): $(APP_BIN)
	python3 -c "import sys; sys.path.insert(0, '../../tools'); import apb1_format; \
	manifest = apb1_format.make_manifest(app_id=b'org.arenaos.sdl3app', package_id=b'org.arenaos.sdl3app', \
	display_name=b'SDL3 Demo App', version=1, flags=25, requested=0, entry=b'bin/sdl3_app', icon=b'', width=320, height=240, associations=()); \
	bundle = apb1_format.build_bundle([(1, b'bin/sdl3_app', open('$^', 'rb').read())], manifest=manifest); \
	open('$@', 'wb').write(bundle)"

clean:
	rm -rf build ../sdl3-app/build

.PHONY: all clean
