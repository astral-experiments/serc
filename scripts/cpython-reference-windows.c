/* Keep the pinned reference startup unchanged while using binary Windows output. */
#include <fcntl.h>
#include <io.h>

#define main reference_main
#include "cpython-reference.c"
#undef main

int main(int argc, char **argv) {
    if (_setmode(_fileno(stdout), _O_BINARY) == -1) {
        perror("stdout");
        return 1;
    }
    return reference_main(argc, argv);
}
