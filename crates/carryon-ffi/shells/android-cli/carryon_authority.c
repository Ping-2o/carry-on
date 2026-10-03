/*
 * carryon_authority.c — an on-device L4 authority-transfer demonstration driven
 * entirely through the Carry-On C ABI (include/carryon.h).
 *
 * Two independent engine cores (two data roots) run in one process on the device.
 * The source seals an editor cut and serves it to the destination over real TLS 1.3
 * on the device's own loopback; then the full §21.2 single-writer authority transfer
 * runs: the source proposes + relinquishes, the destination accepts + becomes owner.
 * We assert ownership flips via carryon_may_mutate on both sides.
 *
 * Trust is built WITHOUT parsing nested pairing JSON in C: each side starts from an
 * empty TrustStore ({"peers":{}}) and pins the peer by its pin-hex.
 *
 * LOCAL/PHYSICAL evidence (spec §2/§30/PLAT-001): proves the authority path runs on
 * real device hardware through the C ABI. Loopback within one device is LOCAL, not
 * cross-device evidence; no platform is claimed "supported" (PLAT-006 out of scope).
 *
 * Exit 0 only if every C-ABI call returns CARRYON_OK and ownership moves.
 *
 * Note: Android's bionic libc provides pthreads in libc itself — do NOT link
 * -lpthread (it does not exist in the NDK sysroot).
 */

#include "carryon.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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
        }                                                                       \
    } while (0)

/* Shared across the source thread and main (destination). Raw pointers are opaque
 * handles; we move them across the thread boundary deliberately. */
struct src_ctx {
    CarryonCore *core;
    CarryonListener *listener;
    CarryonIdentity *id;
    CarryonTrust *trust;
    char session_id[64];
    uint64_t cut_num;
};

static void *source_thread(void *arg) {
    struct src_ctx *c = (struct src_ctx *)arg;
    CarryonSession *sess = carryon_session_accept(c->listener, c->id, c->trust);
    if (!sess) {
        fprintf(stderr, "FAIL source accept\n");
        print_last_error("accept");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_server_negotiate(sess, "[]", &neg), "server_negotiate");
    carryon_string_free(neg);
    CHECK(carryon_serve_cut(c->core, sess), "serve_cut");

    bool owns = false;
    CHECK(carryon_may_mutate(c->core, c->session_id, &owns), "may_mutate(src pre)");
    if (!owns) {
        fprintf(stderr, "FAIL source does not own before transfer\n");
        exit(2);
    }

    char *receipt = NULL;
    CHECK(carryon_serve_authority_transfer(c->core, sess, c->session_id, c->cut_num,
                                           &receipt),
          "serve_authority_transfer");
    carryon_string_free(receipt);

    bool still = true;
    CHECK(carryon_may_mutate(c->core, c->session_id, &still), "may_mutate(src post)");
    if (still) {
        fprintf(stderr, "FAIL source still writable after relinquishment\n");
        exit(2);
    }
    carryon_session_free(sess);
    return NULL;
}

/* Build an empty trust store and pin `peer` into it by pin-hex. */
static CarryonTrust *trust_with_peer(const char *name, const CarryonIdentity *peer) {
    CarryonTrust *t = carryon_trust_from_json("{\"peers\":{}}");
    if (!t) {
        fprintf(stderr, "FAIL empty trust\n");
        exit(2);
    }
    char *pin = NULL;
    CHECK(carryon_identity_pin_hex(peer, &pin), "pin_hex");
    CHECK(carryon_trust_pair(t, name, pin, "t"), "trust_pair");
    carryon_string_free(pin);
    return t;
}

int main(int argc, char **argv) {
    const char *base = (argc > 1) ? argv[1] : "/data/local/tmp/carryon-authority";
    char src_dir[1024], dst_dir[1024];
    snprintf(src_dir, sizeof(src_dir), "%s/src", base);
    snprintf(dst_dir, sizeof(dst_dir), "%s/dst", base);

    uint32_t maj = 0, min = 0;
    carryon_abi_version(&maj, &min);
    printf("Carry-On Android C shell — L4 authority transfer (through the C ABI)\n");
    printf("  ABI version:    %u.%u\n", maj, min);

    CarryonCore *src = carryon_core_open(src_dir);
    CarryonCore *dst = carryon_core_open(dst_dir);
    if (!src || !dst) {
        fprintf(stderr, "FAIL core_open\n");
        exit(2);
    }

    /* Source: single-writer editor, sealed cut. */
    char *info = NULL;
    CHECK(carryon_register_adapter(src, "org.carryon.editor", "{\"sample\":true}",
                                   &info),
          "register src");
    carryon_string_free(info);
    char *sid = NULL;
    CHECK(carryon_create_session(
              src,
              "{\"adapter_id\":\"org.carryon.editor\",\"title\":\"e\","
              "\"privacy\":\"personal\",\"authority_mode\":\"single_writer\"}",
              &sid),
          "create_session");

    char *cut_json = NULL;
    CHECK(carryon_create_cut(src, sid, &cut_json), "create_cut");
    /* Extract the cut number: the JSON begins {"number":N,... */
    uint64_t cut_num = 0;
    {
        const char *p = strstr(cut_json, "\"number\":");
        if (!p) {
            fprintf(stderr, "FAIL no cut number in %s\n", cut_json);
            exit(2);
        }
        cut_num = strtoull(p + 9, NULL, 10);
    }
    carryon_string_free(cut_json);

    /* Destination: editor with the SAME content so the content-hash binding holds. */
    char *dinfo = NULL;
    CHECK(carryon_register_adapter(
              dst, "org.carryon.editor",
              "{\"session\":\"mirror\",\"text\":\"cooperative draft v1\"}", &dinfo),
          "register dst");
    carryon_string_free(dinfo);

    /* Identities + per-side trust (pin the peer). */
    CarryonIdentity *src_id = carryon_identity_generate("source");
    CarryonIdentity *dst_id = carryon_identity_generate("dest");
    if (!src_id || !dst_id) {
        fprintf(stderr, "FAIL identity\n");
        exit(2);
    }
    CarryonTrust *src_trust = trust_with_peer("dest", dst_id);
    CarryonTrust *dst_trust = trust_with_peer("source", src_id);

    /* Listener + address. */
    CarryonListener *listener = carryon_listener_bind("127.0.0.1:0");
    if (!listener) {
        fprintf(stderr, "FAIL bind\n");
        exit(2);
    }
    char *addr = NULL;
    CHECK(carryon_listener_addr(listener, &addr), "listener_addr");

    /* Mirror id the destination will own. */
    char *mirror = NULL;
    CHECK(carryon_mirror_session_id(sid, &mirror), "mirror_session_id");

    /* Start the source thread. */
    struct src_ctx ctx;
    ctx.core = src;
    ctx.listener = listener;
    ctx.id = src_id;
    ctx.trust = src_trust;
    snprintf(ctx.session_id, sizeof(ctx.session_id), "%s", sid);
    ctx.cut_num = cut_num;
    pthread_t th;
    if (pthread_create(&th, NULL, source_thread, &ctx) != 0) {
        fprintf(stderr, "FAIL pthread_create\n");
        exit(2);
    }

    /* Destination: connect, import, then accept authority. */
    CarryonSession *sess = carryon_session_connect(addr, dst_id, dst_trust);
    if (!sess) {
        fprintf(stderr, "FAIL connect\n");
        print_last_error("connect");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_client_negotiate(sess, "[]", &neg), "client_negotiate");
    carryon_string_free(neg);

    char *out = NULL;
    CHECK(carryon_import_cut(dst, sess, sid, cut_num, &out), "import_cut");
    carryon_string_free(out);

    bool pre = true;
    CHECK(carryon_may_mutate(dst, mirror, &pre), "may_mutate(dst pre)");
    if (pre) {
        fprintf(stderr, "FAIL mirror writable before transfer\n");
        exit(2);
    }

    char *receipt = NULL;
    CHECK(carryon_request_authority_transfer(dst, sess, mirror, "org.carryon.editor",
                                             &receipt),
          "request_authority_transfer");
    printf("  receipt set:    %s\n", receipt);
    carryon_string_free(receipt);

    bool post = false;
    CHECK(carryon_may_mutate(dst, mirror, &post), "may_mutate(dst post)");
    if (!post) {
        fprintf(stderr, "FAIL destination not owner after transfer\n");
        exit(2);
    }

    pthread_join(th, NULL);

    printf("  source:         %s\n", sid);
    printf("  mirror (owner): %s\n", mirror);
    printf("  ownership:      moved source -> destination (epoch advanced)\n");
    printf("  DISCLOSURE: L4 authority transfer ran on the device CPU + filesystem "
           "through the C ABI (PLAT-001). Loopback within one device is LOCAL "
           "evidence; no platform 'supported' (spec §2/§30).\n");

    carryon_string_free(sid);
    carryon_string_free(addr);
    carryon_string_free(mirror);
    carryon_session_free(sess);
    carryon_listener_free(listener);
    carryon_identity_free(src_id);
    carryon_identity_free(dst_id);
    carryon_trust_free(src_trust);
    carryon_trust_free(dst_trust);
    carryon_core_free(src);
    carryon_core_free(dst);
    return 0;
}
