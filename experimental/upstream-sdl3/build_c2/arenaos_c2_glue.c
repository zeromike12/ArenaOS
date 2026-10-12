#include <stdlib.h>
/* Resolve POSIX _exit to standard C _Exit provided by libarena_c.a */
void _exit(int status) {
    _Exit(status);
}
