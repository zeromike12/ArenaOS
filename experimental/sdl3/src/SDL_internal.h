#ifndef SDL_internal_h_
#define SDL_internal_h_

#include "build_config/SDL_build_config.h"

#define SDL_PLATFORM_DEFINED 1

#include <SDL3/SDL.h>
#include <SDL3/SDL_intrin.h>

#ifndef SDL_COMPILE_TIME_ASSERT
#define SDL_COMPILE_TIME_ASSERT(name, x) typedef int SDL_compile_time_assert_ ## name[(x) * 2 - 1]
#endif

#ifndef SDL_arraysize
#define SDL_arraysize(array) (sizeof(array)/sizeof((array)[0]))
#endif

#ifndef SDL_zero
#define SDL_zero(x) SDL_memset(&(x), 0, sizeof((x)))
#endif

#ifndef SDL_zerop
#define SDL_zerop(x) SDL_memset((x), 0, sizeof(*(x)))
#endif

#ifndef SDL_zeroa
#define SDL_zeroa(x) SDL_memset((x), 0, sizeof((x)))
#endif

#ifndef SDL_min
#define SDL_min(x, y) (((x) < (y)) ? (x) : (y))
#endif

#ifndef SDL_max
#define SDL_max(x, y) (((x) > (y)) ? (x) : (y))
#endif

#ifndef SDL_clamp
#define SDL_clamp(x, a, b) (((x) < (a)) ? (a) : (((x) > (b)) ? (b) : (x)))
#endif

#endif /* SDL_internal_h_ */
