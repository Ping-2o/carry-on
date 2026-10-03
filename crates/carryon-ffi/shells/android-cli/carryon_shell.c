/*
 * carryon_shell.c — a minimal NON-RUST system shell for the Carry-On container
 * runtime, linking libcarryon_ffi through the C ABI in include/carryon.h.
 *
 * Purpose (spec §2/§3.8/§30): prove the "import a program → carry it → relaunch"
 * lifecycle runs on a real device CPU + filesystem, driven by a plain C program
 * that passes DATA and never behaviour across the boundary. This is the honest
 * portable-program-container shell: it links the runtime and calls it; it does
 * not execute any foreign binary.
 *
 * Cross-compiled for aarch64-linux-android with the NDK, pushed via adb, and run
 * in `adb shell` on a rooted device. Running on physical hardware is the point
 * (PLAT-001); packaging/signing as an APK (PLAT-006) is deliberately out of scope
 * for this shell.
 *
 * Exit code 0 only if every step returns CARRYON_OK and the evidence bundle
 * re-verifies on the device.
 */

#include "carryon.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Print the thread-local last-error message set by the FFI, if any. */
static void print_last_error(const char *where) {
    uint8_t buf[1024];
    size_t len = sizeof(buf);
    int32_t rc = carryon_last_error(buf, &len);
    if (rc == CARRYON_OK && len > 0 && len <= sizeof(buf)) {
        fprintf(stderr, "  [%s] last_error: %.*s\n", where, (int)len, buf);
    } else {
        fprintf(stderr, "  [%s] (no last-error message)\n", where);
    }
}

/* Abort the run with a diagnostic if `rc` is not OK. */
#define CHECK(rc, where)                                                        \
    do {                                                                        \
        int32_t _rc = (rc);                                                     \
        if (_rc != CARRYON_OK) {                                                \
            fprintf(stderr, "FAIL %s: rc=%d\n", (where), _rc);                  \
            print_last_error(where);                                            \
            return 2;                                                           \
        }                                                                       \
    } while (0)

int main(int argc, char **argv) {
    const char *data_dir = (argc > 1) ? argv[1] : "/data/local/tmp/carryon-shell";

    uint32_t maj = 0, min = 0;
    carryon_abi_version(&maj, &min);
    printf("Carry-On Android C shell (through the C ABI)\n");
    printf("  ABI version:    %u.%u (header %u.%u)\n", maj, min,
           CARRYON_ABI_MAJOR, CARRYON_ABI_MINOR);
    if (maj != CARRYON_ABI_MAJOR) {
        fprintf(stderr, "FAIL abi: major mismatch (lib %u vs header %u)\n", maj,
                CARRYON_ABI_MAJOR);
        return 2;
    }

    /* 1. Open the engine rooted on the device filesystem. */
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core) {
        fprintf(stderr, "FAIL core_open at %s\n", data_dir);
        print_last_error("core_open");
        return 2;
    }

    /* 2. Import a program: register a compiled-in adapter by id + data. */
    char *info = NULL;
    CHECK(carryon_register_adapter(core, "org.carryon.graph", "{\"sample\":true}",
                                   &info),
          "register_adapter");
    carryon_string_free(info);

    /* 3. Create a session (one coherent body of work). */
    char *sid = NULL;
    CHECK(carryon_create_session(
              core,
              "{\"adapter_id\":\"org.carryon.graph\",\"title\":\"android shell\","
              "\"privacy\":\"public\",\"authority_mode\":\"read_only_replica\"}",
              &sid),
          "create_session");
    printf("  session:        %s\n", sid);

    /* 4. Seal a cut (immutable, verifiable snapshot of authoritative state). */
    char *cut = NULL;
    CHECK(carryon_create_cut(core, sid, &cut), "create_cut");

    /* 5. Relaunch: run a source-off action against the carried state. */
    char *result = NULL;
    CHECK(carryon_execute_action(
              core, cut,
              "{\"class\":\"graph.shortest_path\",\"params\":{\"start\":0,"
              "\"end\":4}}",
              &result),
          "execute_action");
    printf("  action result:  %s\n", result);

    /* 6. Export an evidence bundle and re-verify it on the device. */
    char *bundle = NULL;
    CHECK(carryon_export_evidence(core, sid, &bundle), "export_evidence");

    char bundle_path[1024];
    snprintf(bundle_path, sizeof(bundle_path), "%s/android-shell-evidence.json",
             data_dir);
    FILE *f = fopen(bundle_path, "wb");
    if (!f) {
        fprintf(stderr, "FAIL write bundle to %s\n", bundle_path);
        return 2;
    }
    fputs(bundle, f);
    fclose(f);

    char *report = NULL;
    CHECK(carryon_verify_evidence(core, bundle_path, &report), "verify_evidence");
    printf("  bundle verify:  %s\n", report);
    printf("  written to:     %s\n", bundle_path);

    carryon_string_free(sid);
    carryon_string_free(cut);
    carryon_string_free(result);
    carryon_string_free(bundle);
    carryon_string_free(report);
    carryon_core_free(core);

    printf("  DISCLOSURE: ran on the device CPU + filesystem through the C ABI "
           "(PLAT-001 physical execution). Not an APK; no platform is 'supported' "
           "on this basis alone (spec §2/§30).\n");
    return 0;
}
