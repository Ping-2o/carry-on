/*
 * carryon_xdev.c — REAL cross-device Carry-On handoff + L4 authority transfer over
 * a LAN TLS 1.3 link, driven through the C ABI (include/carryon.h). One binary, two
 * roles:
 *
 *   source  <id_dir> <data_dir> <bind_addr>   <peer_pin_hex>
 *   dest    <id_dir> <data_dir> <peer_addr>   <peer_pin_hex>
 *   pin     <id_dir>
 *
 * The device identity is PERSISTED under <id_dir> (cert.der + key.der) so its pin is
 * stable across runs (§18.4). `pin` prints the stable pin-hex so the two machines can
 * exchange pins out-of-band before connecting (each pins the other, fail-closed).
 *
 * source: single-writer editor, seals a cut, binds, accepts one peer, serves the cut,
 *         then proposes + relinquishes authority (§21.2). Ends read-only.
 * dest:   editor with matching content, connects to the source, imports the cut, then
 *         accepts authority and becomes the owner.
 *
 * This is PHYSICAL cross-device evidence (PLAT-001): two independent machines, two
 * NICs, real mutual-pinned TLS over the LAN — NOT loopback. It is still not a
 * "supported platform" claim (no APK/signing; PLAT-006 out of scope). The source
 * runs on one machine, the dest on the other; neither drives the peer.
 *
 * Note: Android bionic libc has pthreads built in; do NOT link -lpthread.
 */

#include "carryon.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define EDITOR_ID "org.carryon.editor"
#define EDITOR_TEXT "cooperative draft v1"

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

/* Read a whole file into a malloc'd buffer. Returns bytes read, or -1 if absent. */
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

/* Load the persisted identity from <id_dir>, or generate + persist a fresh one. */
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
            print_last_error("from_der");
            exit(2);
        }
        return id;
    }
    free(cert);
    free(key);

    /* Generate fresh and persist (cert public, key SECRET — local file only here). */
    CarryonIdentity *id = carryon_identity_generate(name);
    if (!id) {
        fprintf(stderr, "FAIL identity_generate\n");
        exit(2);
    }
    /* Query length then copy (the *len=NULL-buf protocol). */
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
    CHECK(carryon_trust_pair(t, name, peer_pin_hex, "xdev"), "trust_pair");
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

static int do_source(const char *id_dir, const char *data_dir, const char *bind_addr,
                     const char *peer_pin) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core) {
        fprintf(stderr, "FAIL core_open\n");
        exit(2);
    }
    char *info = NULL;
    CHECK(carryon_register_adapter(core, EDITOR_ID, "{\"sample\":true}", &info),
          "register");
    carryon_string_free(info);

    char *sid = NULL;
    CHECK(carryon_create_session(
              core,
              "{\"adapter_id\":\"" EDITOR_ID "\",\"title\":\"xdev\","
              "\"privacy\":\"personal\",\"authority_mode\":\"single_writer\"}",
              &sid),
          "create_session");
    char *cut_json = NULL;
    CHECK(carryon_create_cut(core, sid, &cut_json), "create_cut");
    uint64_t cut_num = 0;
    {
        const char *p = strstr(cut_json, "\"number\":");
        if (!p) {
            fprintf(stderr, "FAIL no cut number\n");
            exit(2);
        }
        cut_num = strtoull(p + 9, NULL, 10);
    }
    carryon_string_free(cut_json);

    CarryonIdentity *id = load_or_make_identity(id_dir, "source");
    CarryonTrust *trust = trust_with_pin("dest", peer_pin);
    CarryonListener *listener = carryon_listener_bind(bind_addr);
    if (!listener) {
        fprintf(stderr, "FAIL bind %s\n", bind_addr);
        print_last_error("bind");
        exit(2);
    }
    char *addr = NULL;
    CHECK(carryon_listener_addr(listener, &addr), "listener_addr");
    printf("source: session=%s cut=%llu listening=%s\n", sid,
           (unsigned long long)cut_num, addr);
    printf("source: waiting for destination to connect...\n");
    fflush(stdout);

    CarryonSession *sess = carryon_session_accept(listener, id, trust);
    if (!sess) {
        fprintf(stderr, "FAIL accept (pin mismatch or unpaired?)\n");
        print_last_error("accept");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_server_negotiate(sess, "[]", &neg), "server_negotiate");
    carryon_string_free(neg);

    CHECK(carryon_serve_cut(core, sess), "serve_cut");
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
    printf("source: OK — authority relinquished to destination\n");

    carryon_string_free(sid);
    carryon_string_free(addr);
    carryon_session_free(sess);
    carryon_listener_free(listener);
    carryon_identity_free(id);
    carryon_trust_free(trust);
    carryon_core_free(core);
    return 0;
}

static int do_dest(const char *id_dir, const char *data_dir, const char *peer_addr,
                   const char *peer_pin, const char *remote_session) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core) {
        fprintf(stderr, "FAIL core_open\n");
        exit(2);
    }
    char *info = NULL;
    CHECK(carryon_register_adapter(
              core, EDITOR_ID,
              "{\"session\":\"mirror\",\"text\":\"" EDITOR_TEXT "\"}", &info),
          "register");
    carryon_string_free(info);

    CarryonIdentity *id = load_or_make_identity(id_dir, "dest");
    CarryonTrust *trust = trust_with_pin("source", peer_pin);

    printf("dest: connecting to %s...\n", peer_addr);
    fflush(stdout);
    CarryonSession *sess = carryon_session_connect(peer_addr, id, trust);
    if (!sess) {
        fprintf(stderr, "FAIL connect %s (pin mismatch or unreachable?)\n", peer_addr);
        print_last_error("connect");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_client_negotiate(sess, "[]", &neg), "client_negotiate");
    carryon_string_free(neg);

    /* remote_session arg can be "0" to mean cut 0; we need the source's session id. */
    char *out = NULL;
    uint64_t cut_num = strtoull(getenv("XDEV_CUT") ? getenv("XDEV_CUT") : "0", NULL, 10);
    CHECK(carryon_import_cut(core, sess, remote_session, cut_num, &out), "import_cut");
    printf("dest: imported %s\n", out);
    carryon_string_free(out);

    char *mirror = NULL;
    CHECK(carryon_mirror_session_id(remote_session, &mirror), "mirror_session_id");
    bool pre = true;
    CHECK(carryon_may_mutate(core, mirror, &pre), "may_mutate(pre)");
    printf("dest: may_mutate before transfer = %s\n", pre ? "true" : "false");

    char *receipt = NULL;
    CHECK(carryon_request_authority_transfer(core, sess, mirror, EDITOR_ID, &receipt),
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
    printf("dest: OK — now authoritative owner of %s\n", mirror);

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
                "  %s source <id_dir> <data_dir> <bind_addr> <dest_pin_hex>\n"
                "  %s dest   <id_dir> <data_dir> <source_addr> <source_pin_hex> "
                "<source_session_id>   (env XDEV_CUT=<n>)\n",
                argv[0], argv[0], argv[0]);
        return 1;
    }
    const char *role = argv[1];
    if (strcmp(role, "pin") == 0)
        return do_pin(argv[2]);
    if (strcmp(role, "source") == 0 && argc >= 6)
        return do_source(argv[2], argv[3], argv[4], argv[5]);
    if (strcmp(role, "dest") == 0 && argc >= 7)
        return do_dest(argv[2], argv[3], argv[4], argv[5], argv[6]);
    fprintf(stderr, "bad args\n");
    return 1;
}
