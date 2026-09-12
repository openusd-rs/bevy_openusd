#include <X11/Xlib.h>
#include <stdio.h>
#include <string.h>

static unsigned find_window(Display *display, Window window, const char *title, Window *selected, unsigned depth) {
    if (depth > 32) return 0;
    char *name = NULL;
    unsigned matches = 0, count = 0;
    if (XFetchName(display, window, &name) && name) {
        if (!strcmp(name, title)) { *selected = window; ++matches; }
        XFree(name);
    }
    Window root, parent, *children = NULL;
    if (XQueryTree(display, window, &root, &parent, &children, &count)) {
        for (unsigned i = 0; i < count; ++i)
            matches += find_window(display, children[i], title, selected, depth + 1);
        if (children) XFree(children);
    }
    return matches;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: send_window_close DISPLAY EXACT_WINDOW_TITLE\n");
        return 2;
    }
    Display *display = XOpenDisplay(argv[1]);
    if (!display) { fprintf(stderr, "cannot open display\n"); return 1; }
    Window selected = None;
    unsigned matches = find_window(display, DefaultRootWindow(display), argv[2], &selected, 0);
    Atom protocol = XInternAtom(display, "WM_DELETE_WINDOW", False);
    Atom *protocols = NULL;
    int protocol_count = 0, supported = 0;
    if (matches == 1 && XGetWMProtocols(display, selected, &protocols, &protocol_count)) {
        for (int i = 0; i < protocol_count; ++i) supported |= protocols[i] == protocol;
        XFree(protocols);
    }
    if (matches != 1 || !supported) {
        fprintf(stderr, "expected one matching window supporting WM_DELETE_WINDOW; matches=%u\n", matches);
        XCloseDisplay(display);
        return 1;
    }
    XEvent event = {0};
    event.xclient.type = ClientMessage;
    event.xclient.window = selected;
    event.xclient.message_type = XInternAtom(display, "WM_PROTOCOLS", False);
    event.xclient.format = 32;
    event.xclient.data.l[0] = protocol;
    event.xclient.data.l[1] = CurrentTime;
    int sent = XSendEvent(display, selected, False, NoEventMask, &event);
    XSync(display, False);
    printf("OS_CLOSE_REQUEST window=0x%lx sent=%d\n", selected, sent);
    XCloseDisplay(display);
    return sent ? 0 : 1;
}
