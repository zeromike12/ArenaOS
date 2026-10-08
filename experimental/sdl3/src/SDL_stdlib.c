#include "SDL_internal.h"
#include <stdarg.h>

#undef SDL_malloc
#undef SDL_free
#undef SDL_calloc
#undef SDL_realloc
#undef SDL_memcpy
#undef SDL_memset
#undef SDL_memmove
#undef SDL_memcmp
#undef SDL_strlen
#undef SDL_strcmp
#undef SDL_strncmp

extern void *malloc(size_t size);
extern void free(void *ptr);
extern void *calloc(size_t nmemb, size_t size);
extern void *realloc(void *ptr, size_t size);
extern void *memcpy(void *dest, const void *src, size_t n);
extern void *memset(void *s, int c, size_t n);
extern void *memmove(void *dest, const void *src, size_t n);
extern int memcmp(const void *s1, const void *s2, size_t n);
extern size_t strlen(const char *s);
extern int strcmp(const char *s1, const char *s2);
extern int strncmp(const char *s1, const char *s2, size_t n);

void * SDLCALL SDL_malloc(size_t size) {
    return malloc(size);
}

void SDLCALL SDL_free(void *mem) {
    free(mem);
}

void * SDLCALL SDL_calloc(size_t nmemb, size_t size) {
    return calloc(nmemb, size);
}

void * SDLCALL SDL_realloc(void *mem, size_t size) {
    return realloc(mem, size);
}

size_t SDLCALL SDL_GetSIMDAlignment(void) {
    return 16;
}

void * SDLCALL SDL_aligned_alloc(size_t alignment, size_t size) {
    if (alignment < sizeof(void *)) alignment = sizeof(void *);
    size_t total = size + alignment + sizeof(void *);
    void *raw = malloc(total);
    if (!raw) return NULL;
    uintptr_t ptr = (uintptr_t)raw + sizeof(void *);
    uintptr_t aligned = (ptr + alignment - 1) & ~(alignment - 1);
    ((void **)aligned)[-1] = raw;
    return (void *)aligned;
}

void SDLCALL SDL_aligned_free(void *mem) {
    if (mem) {
        free(((void **)mem)[-1]);
    }
}

void * SDLCALL SDL_memcpy(void *dst, const void *src, size_t len) {
    return memcpy(dst, src, len);
}

void * SDLCALL SDL_memset(void *dst, int c, size_t len) {
    return memset(dst, c, len);
}

void * SDLCALL SDL_memmove(void *dst, const void *src, size_t len) {
    return memmove(dst, src, len);
}

int SDLCALL SDL_memcmp(const void *s1, const void *s2, size_t len) {
    return memcmp(s1, s2, len);
}

size_t SDLCALL SDL_strlen(const char *str) {
    return strlen(str);
}

int SDLCALL SDL_strcmp(const char *str1, const char *str2) {
    return strcmp(str1, str2);
}

int SDLCALL SDL_strncmp(const char *str1, const char *str2, size_t maxlen) {
    return strncmp(str1, str2, maxlen);
}

int SDLCALL SDL_strcasecmp(const char *str1, const char *str2) {
    while (*str1 && *str2) {
        char c1 = *str1;
        char c2 = *str2;
        if (c1 >= 'A' && c1 <= 'Z') c1 += 32;
        if (c2 >= 'A' && c2 <= 'Z') c2 += 32;
        if (c1 != c2) return (unsigned char)c1 - (unsigned char)c2;
        str1++;
        str2++;
    }
    return (unsigned char)*str1 - (unsigned char)*str2;
}

int SDLCALL SDL_strncasecmp(const char *str1, const char *str2, size_t maxlen) {
    for (size_t i = 0; i < maxlen && (*str1 || *str2); i++) {
        char c1 = *str1;
        char c2 = *str2;
        if (c1 >= 'A' && c1 <= 'Z') c1 += 32;
        if (c2 >= 'A' && c2 <= 'Z') c2 += 32;
        if (c1 != c2) return (unsigned char)c1 - (unsigned char)c2;
        str1++;
        str2++;
    }
    return 0;
}

size_t SDLCALL SDL_strlcpy(char *dst, const char *src, size_t maxlen) {
    size_t srclen = strlen(src);
    if (maxlen > 0) {
        size_t copylen = (srclen >= maxlen) ? maxlen - 1 : srclen;
        memcpy(dst, src, copylen);
        dst[copylen] = '\0';
    }
    return srclen;
}

char * SDLCALL SDL_strdup(const char *string) {
    if (!string) return NULL;
    size_t len = strlen(string) + 1;
    char *copy = (char *)malloc(len);
    if (copy) {
        memcpy(copy, string, len);
    }
    return copy;
}

static void format_num(char **out, char *end, unsigned long val, int base, int is_upper, int min_w, char pad) {
    char buf[32];
    int pos = 0;
    const char *digits = is_upper ? "0123456789ABCDEF" : "0123456789abcdef";

    if (val == 0) {
        buf[pos++] = '0';
    } else {
        while (val > 0) {
            buf[pos++] = digits[val % base];
            val /= base;
        }
    }

    while (pos < min_w && pos < (int)sizeof(buf) - 1) {
        buf[pos++] = pad;
    }

    for (int i = pos - 1; i >= 0; i--) {
        if (*out < end - 1) {
            *(*out)++ = buf[i];
        }
    }
}

int SDLCALL SDL_vsnprintf(char *text, size_t maxlen, const char *fmt, va_list ap) {
    if (!text || maxlen == 0) return 0;
    char *out = text;
    char *end = text + maxlen;

    while (*fmt && out < end - 1) {
        if (*fmt != '%') {
            *out++ = *fmt++;
            continue;
        }

        fmt++;
        char pad = ' ';
        int width = 0;
        if (*fmt == '0') {
            pad = '0';
            fmt++;
        }
        while (*fmt >= '0' && *fmt <= '9') {
            width = width * 10 + (*fmt - '0');
            fmt++;
        }

        while (*fmt == 'l' || *fmt == 'z' || *fmt == 'h') {
            fmt++;
        }

        switch (*fmt) {
            case 's': {
                const char *s = va_arg(ap, const char *);
                if (!s) s = "(null)";
                while (*s && out < end - 1) {
                    *out++ = *s++;
                }
                break;
            }
            case 'c': {
                char c = (char)va_arg(ap, int);
                if (out < end - 1) *out++ = c;
                break;
            }
            case 'd':
            case 'i': {
                long val = va_arg(ap, long);
                if (val < 0) {
                    if (out < end - 1) *out++ = '-';
                    val = -val;
                }
                format_num(&out, end, (unsigned long)val, 10, 0, width, pad);
                break;
            }
            case 'u': {
                unsigned long val = va_arg(ap, unsigned long);
                format_num(&out, end, val, 10, 0, width, pad);
                break;
            }
            case 'x': {
                unsigned long val = va_arg(ap, unsigned long);
                format_num(&out, end, val, 16, 0, width, pad);
                break;
            }
            case 'X': {
                unsigned long val = va_arg(ap, unsigned long);
                format_num(&out, end, val, 16, 1, width, pad);
                break;
            }
            case 'p': {
                void *ptr = va_arg(ap, void *);
                if (out < end - 3) {
                    *out++ = '0';
                    *out++ = 'x';
                }
                format_num(&out, end, (uintptr_t)ptr, 16, 0, 0, ' ');
                break;
            }
            case '%': {
                if (out < end - 1) *out++ = '%';
                break;
            }
            default:
                if (out < end - 1) *out++ = *fmt;
                break;
        }
        if (*fmt) fmt++;
    }

    *out = '\0';
    return (int)(out - text);
}

int SDLCALL SDL_snprintf(char *text, size_t maxlen, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int rc = SDL_vsnprintf(text, maxlen, fmt, ap);
    va_end(ap);
    return rc;
}
