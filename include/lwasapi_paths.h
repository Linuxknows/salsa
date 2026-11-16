#ifndef LWASAPI_PATHS_H
#define LWASAPI_PATHS_H

#include <errno.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static inline bool lwasapi_is_safe_owner_dir(const char *path, uid_t uid, bool require_private) {
    struct stat st;
    if (!path || path[0] != '/') return false;
    if (lstat(path, &st) != 0 || !S_ISDIR(st.st_mode)) return false;
    if (st.st_uid != uid) return false;
    if (require_private && (st.st_mode & 0077) != 0) return false;
    return true;
}

static inline bool lwasapi_ensure_private_dir(char *out, size_t out_len) {
    if (!out || out_len < 32) return false;
    uid_t uid = getuid();
    const char *xdg = getenv("XDG_RUNTIME_DIR");
    char base[256] = {0};

    if (xdg && xdg[0] == '/' && lwasapi_is_safe_owner_dir(xdg, uid, true)) {
        snprintf(base, sizeof(base), "%s", xdg);
    } else {
        char run_fb[64];
        snprintf(run_fb, sizeof(run_fb), "/run/user/%u", (unsigned)uid);
        if (lwasapi_is_safe_owner_dir(run_fb, uid, true)) {
            snprintf(base, sizeof(base), "%s", run_fb);
        }
    }

    if (base[0]) {
        snprintf(out, out_len, "%s/lwasapi", base);
        if (mkdir(out, 0700) == 0 || errno == EEXIST) {
            struct stat st;
            if (lstat(out, &st) == 0 && S_ISDIR(st.st_mode) && st.st_uid == uid) {
                if ((st.st_mode & 0777) != 0700) (void)chmod(out, 0700);
                return true;
            }
        }
    }

    snprintf(out, out_len, "/tmp/lwasapi-%u", (unsigned)uid);
    if (mkdir(out, 0700) == 0 || errno == EEXIST) {
        struct stat st;
        if (lstat(out, &st) == 0 && S_ISDIR(st.st_mode) && st.st_uid == uid) {
            if ((st.st_mode & 0777) != 0700) (void)chmod(out, 0700);
            return true;
        }
    }
    return false;
}

static inline void lwasapi_sock_path_common(char *out, size_t out_len,
                                            const char *env_var,
                                            const char *sock_name,
                                            const char *fallback) {
    if (!out || out_len == 0) return;
    const char *env = env_var ? getenv(env_var) : NULL;
    if (env && env[0]) {
        snprintf(out, out_len, "%s", env);
        return;
    }
    char dir[256];
    if (lwasapi_ensure_private_dir(dir, sizeof(dir))) {
        size_t dlen = strlen(dir);
        size_t slen = strlen(sock_name);
        if (dlen + 1u + slen < out_len) {
            memcpy(out, dir, dlen);
            out[dlen] = '/';
            memcpy(out + dlen + 1u, sock_name, slen + 1u);
            return;
        }
    }
    snprintf(out, out_len, "%s", fallback);
}

static inline void lwasapi_audiodg_sock_path(char *out, size_t out_len, const char *fallback) {
    lwasapi_sock_path_common(out, out_len, "LWASAPI_SOCKET", "audiodg.sock",
                             fallback ? fallback : "/tmp/lwasapi-audiodg.sock");
}

static inline void lwasapi_debug_sock_path(char *out, size_t out_len, const char *fallback) {
    lwasapi_sock_path_common(out, out_len, "LWASAPI_DEBUG_SOCK", "debug.sock",
                             fallback ? fallback : "/tmp/lwasapi-debug.sock");
}

static inline void lwasapi_debug_in_sock_path(char *out, size_t out_len, const char *fallback) {
    lwasapi_sock_path_common(out, out_len, "LWASAPI_DEBUG_IN_SOCK", "debug-in.sock",
                             fallback ? fallback : "/tmp/lwasapi-debug-in.sock");
}

#endif /* LWASAPI_PATHS_H */
