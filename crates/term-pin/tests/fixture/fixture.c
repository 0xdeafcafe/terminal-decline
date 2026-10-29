/* A stand-in for libterminal_decline.dylib in term-pin's tests and the
   P-0 spike (docs/design/pins.md §9). Writes what it was called with to
   the file named by the pin's argv[1] (or the launcher's).

   -DBUILD='"B"'  what it reports as its build
   -DNO_V1        export td_pin_main_v2 only (a newer library a v1 stub
                  can't use)
   -DPROBE        also report whether ~/Documents, ~/Library/Mail and
                  ~/Library/Safari can be read (spike/permissions.sh)
   -DLAYOUT       instead: a program printing the header's layout
   -DINJECT       instead: a library that, when loaded, writes the file
                  named by TD_FIXTURE_INJECTED (DYLD_INSERT_LIBRARIES) */

#include "../../abi/terminal_decline_pin.h"

#include <dirent.h>
#include <errno.h>
#include <pwd.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#ifndef BUILD
#define BUILD "A"
#endif

#if defined(LAYOUT)

int main(void) {
    printf("size=%zu\n", sizeof(struct td_pin_info_v1));
    printf("size_off=%zu\nabi_off=%zu\npin_id_off=%zu\npin_bundle_off=%zu\n",
           offsetof(struct td_pin_info_v1, size), offsetof(struct td_pin_info_v1, abi),
           offsetof(struct td_pin_info_v1, pin_id), offsetof(struct td_pin_info_v1, pin_bundle));
    printf("app_bundle_off=%zu\nstub_version_off=%zu\nargc_off=%zu\nargv_off=%zu\n",
           offsetof(struct td_pin_info_v1, app_bundle), offsetof(struct td_pin_info_v1, stub_version),
           offsetof(struct td_pin_info_v1, argc), offsetof(struct td_pin_info_v1, argv));
    return 0;
}

#elif defined(INJECT)

__attribute__((constructor)) static void injected(void) {
    const char *p = getenv("TD_FIXTURE_INJECTED");
    FILE *f = p ? fopen(p, "w") : NULL;
    if (f) {
        fputs("injected\n", f);
        fclose(f);
    }
}

#else

static FILE *out(int argc, const char *const *argv) {
    return argc > 1 ? fopen(argv[1], "w") : NULL;
}

#ifndef NO_V1
int td_pin_main_v1(const struct td_pin_info_v1 *info) {
    FILE *f = out(info->argc, info->argv);
    if (!f)
        return 3;
    fprintf(f, "entry=td_pin_main_v1\nbuild=%s\nsize=%u\nabi=%u\n", BUILD, info->size, info->abi);
    fprintf(f, "pin_id=%s\npin_bundle=%s\napp_bundle=%s\nstub_version=%s\nargc=%d\n", info->pin_id,
            info->pin_bundle, info->app_bundle, info->stub_version, info->argc);
    for (int i = 0; i < info->argc; i++)
        fprintf(f, "argv=%s\n", info->argv[i]);
#ifdef PROBE
    /* The spike's permission probe (spike/permissions.sh): can this pin
       read protected folders? From the password database, since the
       stub's HOME may be a test one. */
    const char *home = getpwuid(getuid())->pw_dir;
    const char *dirs[] = {"Documents", "Library/Mail", "Library/Safari"};
    for (int d = 0; d < 3; d++) {
        char p[1024];
        snprintf(p, sizeof p, "%s/%s", home, dirs[d]);
        DIR *dir = opendir(p);
        fprintf(f, "%s=%s\n", dirs[d], dir ? "readable" : strerror(errno));
        if (dir)
            closedir(dir);
    }
#endif
    fclose(f);
    return 7;
}
#endif

/* Any newer struct: a v1 stub must never call this. */
int td_pin_main_v2(const void *info) {
    (void)info;
    return 8;
}

int td_app_main_v1(int argc, const char *const *argv) {
    FILE *f = out(argc, argv);
    if (!f)
        return 3;
    fprintf(f, "entry=td_app_main_v1\nbuild=%s\nargc=%d\n", BUILD, argc);
    for (int i = 0; i < argc; i++)
        fprintf(f, "argv=%s\n", argv[i]);
    fclose(f);
    return 9;
}

#endif
