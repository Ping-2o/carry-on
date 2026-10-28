/*
 * carryon_kiwix.c — REAL cross-device Kiwix reading-session carry + L4 authority
 * transfer over a LAN TLS 1.3 link, driven through the Carry-On C ABI. One
 * binary, two roles (the carryon_xdev.c pattern, kiwix edition):
 *
 *   pin    <id_dir>
 *   source <id_dir> <data_dir> <bind_addr> <dest_pin_hex> \
 *          <zim_path> <article_path> <scroll_y> [strategy D|A]
 *   dest   <id_dir> <data_dir> <src_addr> <src_pin_hex> <source_session_id> \
 *          <zim_path> <article_path> <scroll_y> <navigate_path>
 *          (env KIWIX_CUT=<n>, default 0)
 *
 * Both machines hold the SAME ZIM file (each opens it locally through the
 * libzim backend — no fork of the Kiwix app; the ZIM is just a file). The cut
 * carries the session identity + reading position + current article (KBs).
 *
 * source: opens the real ZIM, seals a cut, binds, accepts one peer, serves the
 *         cut, then proposes + relinquishes authority (§21.2). Ends read-only.
 * dest:   opens the SAME ZIM, connects, imports the cut (progressive: only the
 *         sealed objects — the 27 MB bulk stays behind as an unsealed Cache
 *         object), prints the REAL wire counters (carryon_session_bytes),
 *         runs reader.restore, accepts authority, and — the L4 story —
 *         executes reader.navigate on ITS OWN device CPU afterwards: the
 *         reading position physically moved.
 *
 * PHYSICAL cross-device evidence (PLAT-001): two independent machines, two
 * NICs, real mutual-pinned TLS over the LAN. Not an APK; no platform
 * "supported" claim (PLAT-006 / §30 out of scope).
 *
 * Requires libcarryon_ffi built with --features kiwix.
 * Note: Android bionic libc has pthreads built in; do NOT link -lpthread.
 */

#include "carryon.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define KIWIX_ID "org.carryon.kiwix"

static void print_last_error(const char *where) {
    uint8_t buf[1024];
    size_t len = sizeof(buf);
    if (carryon_last_error(buf, &len) == CARRYON_OK && len > 0 && len <= sizeof(buf))
        fprintf(stderr, "  [%s] last_error: %.*s\n", where, (int)len, buf);
}

#define CHECK(rc, where)                                                        \
    do {                                                                        \
        int32_t _rc = (rc);                                                     \
        if (_rc != CARRYON_OK) {                                                \
            fprintf(stderr, "FAIL %s: rc=%d\n", (where), _rc);                  \
            print_last_error(where);                                            \
            exit(2);                                                            \
        }                                                                        \
    } while (0)

static long read_file(const char *path, uint8_t **out) {
    FILE *f = fopen(path, "rb");
    if (!f)
        return -1;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *b = malloc(n > 0 ? (size_t)n : 1);
    if (fread(b, 1, (size_t)n, f) != (size_t)n) {
        fclose(f);
        free(b);
        return -1;
    }
    fclose(f);
    *out = b;
    return n;
}

static void write_file(const char *path, const uint8_t *b, size_t n) {
    FILE *f = fopen(path, "wb");
    if (!f) {
        fprintf(stderr, "FAIL write %s\n", path);
        exit(2);
    }
    fwrite(b, 1, n, f);
    fclose(f);
}

static CarryonIdentity *load_or_make_identity(const char *id_dir, const char *name) {
    char cert_path[1024], key_path[1024];
    snprintf(cert_path, sizeof(cert_path), "%s/cert.der", id_dir);
    snprintf(key_path, sizeof(key_path), "%s/key.der", id_dir);

    uint8_t *cert = NULL, *key = NULL;
    long cert_n = read_file(cert_path, &cert);
    long key_n = read_file(key_path, &key);
    if (cert_n > 0 && key_n > 0) {
        CarryonIdentity *id = carryon_identity_from_der(name, cert, (size_t)cert_n,
                                                        key, (size_t)key_n);
        free(cert);
        free(key);
        if (!id) {
            fprintf(stderr, "FAIL identity_from_der\n");
            exit(2);
        }
        return id;
    }
    free(cert);
    free(key);

    CarryonIdentity *id = carryon_identity_generate(name);
    if (!id) {
        fprintf(stderr, "FAIL identity_generate\n");
        exit(2);
    }
    size_t cl = 0, kl = 0;
    carryon_identity_cert_der(id, NULL, &cl);
    carryon_identity_key_der(id, NULL, &kl);
    uint8_t *cb = malloc(cl), *kb = malloc(kl);
    CHECK(carryon_identity_cert_der(id, cb, &cl), "cert_der");
    CHECK(carryon_identity_key_der(id, kb, &kl), "key_der");
    write_file(cert_path, cb, cl);
    write_file(key_path, kb, kl);
    free(cb);
    free(kb);
    return id;
}

static CarryonTrust *trust_with_pin(const char *name, const char *peer_pin_hex) {
    CarryonTrust *t = carryon_trust_from_json("{\"peers\":{}}");
    if (!t) {
        fprintf(stderr, "FAIL empty trust\n");
        exit(2);
    }
    CHECK(carryon_trust_pair(t, name, peer_pin_hex, "kiwix"), "trust_pair");
    return t;
}

static int do_pin(const char *id_dir) {
    CarryonIdentity *id = load_or_make_identity(id_dir, "device");
    char *pin = NULL;
    CHECK(carryon_identity_pin_hex(id, &pin), "pin_hex");
    printf("%s\n", pin);
    carryon_string_free(pin);
    carryon_identity_free(id);
    return 0;
}

static uint64_t cut_number_from_json(const char *cut_json) {
    const char *p = strstr(cut_json, "\"number\":");
    if (!p) {
        fprintf(stderr, "FAIL no cut number\n");
        exit(2);
    }
    return strtoull(p + 9, NULL, 10);
}

static void print_wire(const char *who, CarryonSession *sess) {
    uint64_t sent = 0, recv = 0;
    CHECK(carryon_session_bytes(sess, &sent, &recv), "session_bytes");
    printf("%s: WIRE sent=%llu recv=%llu bytes\n", who, (unsigned long long)sent,
           (unsigned long long)recv);
    fflush(stdout);
}

/* ---------------------------------------------------------------- source -- */

static int do_source(const char *id_dir, const char *data_dir, const char *bind_addr,
                     const char *peer_pin, const char *zim_path, const char *article_path,
                     long scroll_y, const char *strategy) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core)
        exit(2);

    char params[2048];
    snprintf(params, sizeof(params),
             "{\"zim_path\":\"%s\",\"article_path\":\"%s\",\"scroll_y\":%ld,"
             "\"strategy\":\"%s\",\"session\":\"kiwix-src\"}",
             zim_path, article_path, scroll_y, strategy);
    char *info = NULL;
    CHECK(carryon_register_adapter(core, KIWIX_ID, params, &info), "register");
    carryon_string_free(info);

    char *sid = NULL;
    CHECK(carryon_create_session(
              core,
              "{\"adapter_id\":\"" KIWIX_ID "\",\"title\":\"kiwix reading\","
              "\"privacy\":\"personal\",\"authority_mode\":\"single_writer\"}",
              &sid),
          "create_session");

    char *cut_json = NULL;
    CHECK(carryon_create_cut(core, sid, &cut_json), "create_cut");
    uint64_t cut_num = cut_number_from_json(cut_json);
    carryon_string_free(cut_json);
    printf("source: strategy=%s session=%s cut=%llu\n", strategy, sid,
           (unsigned long long)cut_num);
    fflush(stdout);

    CarryonIdentity *id = load_or_make_identity(id_dir, "source");
    CarryonTrust *trust = trust_with_pin("dest", peer_pin);
    CarryonListener *listener = carryon_listener_bind(bind_addr);
    if (!listener) {
        print_last_error("bind");
        exit(2);
    }
    char *addr = NULL;
    CHECK(carryon_listener_addr(listener, &addr), "listener_addr");
    printf("source: listening=%s waiting for destination...\n", addr);
    fflush(stdout);

    CarryonSession *sess = carryon_session_accept(listener, id, trust);
    if (!sess) {
        print_last_error("accept");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_server_negotiate(sess, "[]", &neg), "server_negotiate");
    carryon_string_free(neg);

    CHECK(carryon_serve_cut(core, sess), "serve_cut");
    print_wire("source(after cut)", sess);
    printf("source: cut served\n");

    bool owns = false;
    CHECK(carryon_may_mutate(core, sid, &owns), "may_mutate(pre)");
    printf("source: may_mutate before transfer = %s\n", owns ? "true" : "false");

    char *receipt = NULL;
    CHECK(carryon_serve_authority_transfer(core, sess, sid, cut_num, &receipt),
          "serve_authority_transfer");
    printf("source: receipt=%s\n", receipt);
    carryon_string_free(receipt);

    bool still = true;
    CHECK(carryon_may_mutate(core, sid, &still), "may_mutate(post)");
    printf("source: may_mutate after relinquishment = %s\n", still ? "true" : "false");
    if (still) {
        fprintf(stderr, "FAIL source still writable\n");
        exit(2);
    }
    print_wire("source(final)", sess);
    printf("source: OK — authority relinquished; reading position moved to destination\n");

    carryon_string_free(sid);
    carryon_string_free(addr);
    carryon_session_free(sess);
    carryon_listener_free(listener);
    carryon_identity_free(id);
    carryon_trust_free(trust);
    carryon_core_free(core);
    return 0;
}

/* ------------------------------------------------------------------ dest -- */

static int do_dest(const char *id_dir, const char *data_dir, const char *peer_addr,
                   const char *peer_pin, const char *remote_session, const char *zim_path,
                   const char *article_path, long scroll_y, const char *navigate_path) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core)
        exit(2);

    /* Same ZIM, same captured position: the destination reconstructs the
     * logical session locally (the cut's digests verify the equivalence). */
    char params[2048];
    snprintf(params, sizeof(params),
             "{\"zim_path\":\"%s\",\"article_path\":\"%s\",\"scroll_y\":%ld,"
             "\"strategy\":\"D\",\"session\":\"kiwix-dst\"}",
             zim_path, article_path, scroll_y);
    char *info = NULL;
    CHECK(carryon_register_adapter(core, KIWIX_ID, params, &info), "register");
    carryon_string_free(info);

    CarryonIdentity *id = load_or_make_identity(id_dir, "dest");
    CarryonTrust *trust = trust_with_pin("source", peer_pin);

    printf("dest: connecting to %s...\n", peer_addr);
    fflush(stdout);
    CarryonSession *sess = carryon_session_connect(peer_addr, id, trust);
    if (!sess) {
        print_last_error("connect");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_client_negotiate(sess, "[]", &neg), "client_negotiate");
    carryon_string_free(neg);

    uint64_t cut_num = strtoull(getenv("KIWIX_CUT") ? getenv("KIWIX_CUT") : "0", NULL, 10);
    char *out = NULL;
    CHECK(carryon_import_cut(core, sess, remote_session, cut_num, &out), "import_cut");
    printf("dest: imported %s\n", out);
    carryon_string_free(out);
    print_wire("dest(after import)", sess);

    /* First action from the transferred cut: reader.restore. */
    char *mirror = NULL;
    CHECK(carryon_mirror_session_id(remote_session, &mirror), "mirror_session_id");
    char cut_ref[512];
    snprintf(cut_ref, sizeof(cut_ref),
             "{\"session\":\"%s\",\"cut_number\":%llu}", mirror,
             (unsigned long long)cut_num);
    char *result = NULL;
    CHECK(carryon_execute_action_as(
              core, cut_ref,
              "{\"class\":\"reader.restore\",\"params\":{}}", KIWIX_ID, &result),
          "reader.restore");
    printf("dest: RESTORE %s\n", result);
    carryon_string_free(result);

    bool pre = true;
    CHECK(carryon_may_mutate(core, mirror, &pre), "may_mutate(pre)");
    printf("dest: may_mutate before transfer = %s\n", pre ? "true" : "false");

    char *receipt = NULL;
    CHECK(carryon_request_authority_transfer(core, sess, mirror, KIWIX_ID, &receipt),
          "request_authority_transfer");
    printf("dest: receipt=%s\n", receipt);
    carryon_string_free(receipt);

    bool post = false;
    CHECK(carryon_may_mutate(core, mirror, &post), "may_mutate(post)");
    printf("dest: may_mutate after transfer = %s\n", post ? "true" : "false");
    if (!post) {
        fprintf(stderr, "FAIL dest not owner\n");
        exit(2);
    }

    /* THE L4 story: navigate (move the reading position) on the destination's
     * own CPU, now that IT holds authority. The article bytes come from the
     * device's local ZIM through the libzim backend. */
    char nav_req[2048];
    snprintf(nav_req, sizeof(nav_req),
             "{\"class\":\"reader.navigate\",\"params\":{\"path\":\"%s\",\"scroll_y\":321}}",
             navigate_path);
    CHECK(carryon_execute_action_as(core, cut_ref, nav_req, KIWIX_ID, &result),
          "reader.navigate");
    printf("dest: NAVIGATE %s\n", result);
    carryon_string_free(result);

    char *result2 = NULL;
    CHECK(carryon_execute_action_as(
              core, cut_ref,
              "{\"class\":\"reader.restore\",\"params\":{}}", KIWIX_ID, &result2),
          "reader.restore(after navigate)");
    printf("dest: RESTORE-AFTER-NAVIGATE %s\n", result2);
    carryon_string_free(result2);

    print_wire("dest(final)", sess);
    printf("dest: OK — authoritative owner; position moved on-device\n");

    carryon_string_free(mirror);
    carryon_session_free(sess);
    carryon_identity_free(id);
    carryon_trust_free(trust);
    carryon_core_free(core);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr,
                "usage:\n"
                "  %s pin    <id_dir>\n"
                "  %s source <id_dir> <data_dir> <bind_addr> <dest_pin_hex> "
                "<zim_path> <article_path> <scroll_y> [strategy D|A]\n"
                "  %s dest   <id_dir> <data_dir> <source_addr> <source_pin_hex> "
                "<source_session_id> <zim_path> <article_path> <scroll_y> "
                "<navigate_path>   (env KIWIX_CUT=<n>)\n",
                argv[0], argv[0], argv[0]);
        return 1;
    }
    const char *role = argv[1];
    if (strcmp(role, "pin") == 0)
        return do_pin(argv[2]);
    if (strcmp(role, "source") == 0 && argc >= 10)
        return do_source(argv[2], argv[3], argv[4], argv[5], argv[6], argv[7],
                         strtol(argv[8], NULL, 10), argv[9]);
    if (strcmp(role, "dest") == 0 && argc >= 11)
        return do_dest(argv[2], argv[3], argv[4], argv[5], argv[6], argv[7], argv[8],
                       strtol(argv[9], NULL, 10), argv[10]);
    fprintf(stderr, "bad args\n");
    return 1;
}
