#include <X11/Xlib.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>

static unsigned long number(const char *text, unsigned long maximum) {
    char *end;
    errno = 0;
    unsigned long value = strtoul(text, &end, 0);
    if (errno || end == text || *end || !value || value > maximum) return 0;
    return value;
}

int main(int argc, char **argv) {
    if (argc != 5) {
        fprintf(stderr, "usage: resize_window DISPLAY WINDOW_ID WIDTH HEIGHT\n");
        return 2;
    }
    unsigned long window = number(argv[2], ~0UL);
    unsigned long width = number(argv[3], 8192), height = number(argv[4], 8192);
    if (!window || !width || !height || argv[2][0] == '-') return 2;
    Display *display = XOpenDisplay(argv[1]);
    if (!display) { fprintf(stderr, "cannot open display\n"); return 1; }
    XResizeWindow(display, window, (unsigned)width, (unsigned)height);
    XSync(display, False);
    printf("RESIZE_REQUEST window=0x%lx size=%lux%lu\n", window, width, height);
    XCloseDisplay(display);
    return 0;
}
