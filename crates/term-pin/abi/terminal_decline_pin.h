/* The interface between a Terminal Decline pin's stub and the Terminal
   Decline library (libterminal_decline.dylib), docs/design/pins.md §3.5.
   Mirrored by `term_pin::abi`; a test compiles this header and checks the
   two agree.

   Frozen from the first public build that makes stub pins (0.1.0). New
   versions add a new struct and a new function; these are never changed
   or removed, and the library exports every version that has ever been in
   a public build. A stub of version N looks up td_pin_main_vN, then N - 1,
   ... down to 1, and calls the first it finds.

   All pointers stay valid for the life of the process; paths are bytes
   (not necessarily UTF-8); the function is called on the main thread.

   What else the stub freezes is listed in crates/term-pin/FROZEN.md. */

#ifndef TERMINAL_DECLINE_PIN_H
#define TERMINAL_DECLINE_PIN_H

#include <stdint.h>

#define TD_PIN_ABI_V1 1

struct td_pin_info_v1 {
    uint32_t size;               /* sizeof(struct td_pin_info_v1): a check, not a version */
    uint32_t abi;                /* 1 */
    const char *pin_id;          /* "rush" (the pin's TDPinId) */
    const char *pin_bundle;      /* path of the pin's .app */
    const char *app_bundle;      /* path of the Terminal Decline.app the library came from */
    const char *stub_version;    /* the pin's CFBundleShortVersionString */
    int argc;                    /* the pin process's arguments, as the stub got them */
    const char *const *argv;
};

/* Returns the process's exit status (it may also exit itself). */
int td_pin_main_v1(const struct td_pin_info_v1 *info);

/* Not frozen: the app's own launcher (Contents/MacOS/terminal-decline)
   ships in the same bundle as the library, so this one may change with
   any release. */
int td_app_main_v1(int argc, const char *const *argv);

#endif
