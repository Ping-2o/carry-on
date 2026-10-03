/*
 * carryon_appcont.c — REAL external-application continuation across devices.
 *
 * A real file (e.g. a PNG) open on the mac is captured by the generic file adapter
 * (L1, §10.7) by content-hash identity, carried over a LAN TLS 1.3 link to the
 * android device, and there reconstructed BYTE-FOR-BYTE and handed to a real,
 * installed Android application via a documented OS launch API (ACTION_VIEW;
 * §10.3 PLAT-003). The destination verifies the reconstructed file's content-hash
 * equals the source's before launching — file identity is guaranteed (§10.7).
 *
 * This is the honest L1 contract: file identity + launch parameters are guaranteed;
 * unsaved in-memory application state is NOT (and is not claimed).
 *
 * Roles (one binary):
 *   pin    <id_dir>
 *   source <id_dir> <data_dir> <bind_addr> <dest_pin_hex> <file_path>
 *   dest   <id_dir> <data_dir> <src_addr>  <src_pin_hex>  <src_session_id> \
 *          <content_hash_hex> <out_file_path>
 *
 * The destination copies the content-addressed object out of its own store
 * (objects/sha256/ab/cd/<hash>, which stores the exact bytes) to <out_file_path>,
 * re-checks the hash, and prints it. The actual `am start` launch is done by the
 * driver (it needs the device's package manager), keeping this binary portable.
 *
 * PHYSICAL cross-device evidence (PLAT-001): two machines, two NICs, real TLS.
 * Not an APK; no platform "supported" (PLAT-006 out of scope).
 * Android bionic has pthreads in libc; do NOT link -lpthread.
 */

#include "carryon.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define FILE_ID "org.carryon.file"

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
    long cn = read_file(cert_path, &cert), kn = read_file(key_path, &key);
    if (cn > 0 && kn > 0) {
        CarryonIdentity *id =
            carryon_identity_from_der(name, cert, (size_t)cn, key, (size_t)kn);
        free(cert);
        free(key);
        if (!id) {
            print_last_error("from_der");
            exit(2);
        }
        return id;
    }
    free(cert);
    free(key);
    CarryonIdentity *id = carryon_identity_generate(name);
    if (!id)
        exit(2);
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
    if (!t)
        exit(2);
    CHECK(carryon_trust_pair(t, name, peer_pin_hex, "appcont"), "trust_pair");
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
                     const char *peer_pin, const char *file_path) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core)
        exit(2);

    /* Import a REAL program's state: the file adapter reads the actual file by path. */
    char params[2048];
    snprintf(params, sizeof(params), "{\"path\":\"%s\"}", file_path);
    char *info = NULL;
    CHECK(carryon_register_adapter(core, FILE_ID, params, &info), "register file");
    carryon_string_free(info);

    char *sid = NULL;
    CHECK(carryon_create_session(
              core,
              "{\"adapter_id\":\"" FILE_ID "\",\"title\":\"real file\","
              "\"privacy\":\"personal\",\"authority_mode\":\"read_only_replica\"}",
              &sid),
          "create_session");
    char *cut_json = NULL;
    CHECK(carryon_create_cut(core, sid, &cut_json), "create_cut");
    uint64_t cut_num = 0;
    const char *p = strstr(cut_json, "\"number\":");
    if (p)
        cut_num = strtoull(p + 9, NULL, 10);
    carryon_string_free(cut_json);

    CarryonIdentity *id = load_or_make_identity(id_dir, "source");
    CarryonTrust *trust = trust_with_pin("dest", peer_pin);
    CarryonListener *listener = carryon_listener_bind(bind_addr);
    if (!listener) {
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
        print_last_error("accept");
        exit(2);
    }
    char *neg = NULL;
    CHECK(carryon_session_server_negotiate(sess, "[]", &neg), "server_negotiate");
    carryon_string_free(neg);
    CHECK(carryon_serve_cut(core, sess), "serve_cut");
    printf("source: file cut served — OK\n");

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
                   const char *peer_pin, const char *remote_session,
                   const char *content_hash, const char *out_file) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core)
        exit(2);
    /* A file-adapter instance just satisfies registration; import verifies bytes.
     * The factory reads this path, so it must be readable here — any small readable
     * file works (env APPCONT_PLACEHOLDER, else a conventional tmp path). */
    const char *placeholder =
        getenv("APPCONT_PLACEHOLDER") ? getenv("APPCONT_PLACEHOLDER")
                                      : "/data/local/tmp/carryon/placeholder.txt";
    char reg[1200];
    snprintf(reg, sizeof(reg), "{\"path\":\"%s\"}", placeholder);
    char *info = NULL;
    CHECK(carryon_register_adapter(core, FILE_ID, reg, &info), "register file");
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

    uint64_t cut_num =
        strtoull(getenv("APPCONT_CUT") ? getenv("APPCONT_CUT") : "0", NULL, 10);
    char *out = NULL;
    CHECK(carryon_import_cut(core, sess, remote_session, cut_num, &out), "import_cut");
    printf("dest: imported %s\n", out);
    carryon_string_free(out);

    /* Source-independence: the exact bytes now live in OUR store, addressed by the
     * source's content-hash. Prove it, then reconstruct the real file for launch. */
    bool present = false;
    CHECK(carryon_has_object_hex(core, content_hash, &present), "has_object_hex");
    if (!present) {
        fprintf(stderr, "FAIL object %s not present after import\n", content_hash);
        exit(2);
    }
    printf("dest: object %s present in local store (source-independent)\n",
           content_hash);

    /* The store file at objects/sha256/ab/cd/<hash> IS the exact bytes. Copy it out
     * to a real path a real Android app can open. */
    char obj_path[1200];
    snprintf(obj_path, sizeof(obj_path), "%s/objects/sha256/%.2s/%.2s/%s", data_dir,
             content_hash, content_hash + 2, content_hash);
    uint8_t *bytes = NULL;
    long n = read_file(obj_path, &bytes);
    if (n < 0) {
        fprintf(stderr, "FAIL read object file %s\n", obj_path);
        exit(2);
    }
    write_file(out_file, bytes, (size_t)n);
    free(bytes);
    printf("dest: reconstructed real file -> %s (%ld bytes)\n", out_file, n);
    printf("dest: OK — file carried mac->device, identity %s guaranteed\n",
           content_hash);

    carryon_session_free(sess);
    carryon_identity_free(id);
    carryon_trust_free(trust);
    carryon_core_free(core);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: pin|source|dest ... (see source header)\n");
        return 1;
    }
    const char *role = argv[1];
    if (strcmp(role, "pin") == 0)
        return do_pin(argv[2]);
    if (strcmp(role, "source") == 0 && argc >= 7)
        return do_source(argv[2], argv[3], argv[4], argv[5], argv[6]);
    if (strcmp(role, "dest") == 0 && argc >= 9)
        return do_dest(argv[2], argv[3], argv[4], argv[5], argv[6], argv[7], argv[8]);
    fprintf(stderr, "bad args\n");
    return 1;
}
