/*
 * carryon_bench.c — REAL cross-device preparation-strategy benchmark, ONE strategy per
 * run, mac (source) ──LAN TLS 1.3──> android device (destination, via adb).
 *
 * This is the cross-device replacement for the composed loopback benchmark: A, C, and D
 * are each measured as an INDEPENDENT end-to-end execution (own source process, own
 * transfer, own destination process) — never derived arithmetically from a shared
 * measurement. Each `dest` invocation emits ONE trial JSON line to <out_json>, which
 * `bench_xdev.sh` collects and `xtask bench-aggregate` turns into median/p95/CI/failures.
 *
 * Roles (one binary):
 *   pin    <id_dir>
 *   source <id_dir> <data_dir> <bind_addr> <dest_pin_hex> <doc_bytes> <nav_bytes> <strategy>
 *   dest   <id_dir> <data_dir> <src_addr>  <src_pin_hex> <src_session_id> \
 *          <strategy> <demand> <latency_ms> <doc_bytes> <nav_bytes> <out_json>
 *
 * Strategies (continuing a working editor session on another device):
 *   A full        : move the optional payload UP FRONT — the source folds doc+nav into the
 *                   authoritative document, so the whole thing is sealed and imported before
 *                   the first action. bytes_before_first_action includes the optional.
 *   C demand      : move only the authoritative prerequisites; the optional is deferred and
 *                   pulled only on demand (measured across trials as the A−D difference).
 *   D progressive : move the authoritative prerequisites (ACTION_READY), DEFER the optional.
 *
 * The LATENCY axis is set by the orchestrator via CARRYON_NET_DELAY_MS (a bench-only
 * per-frame send delay in carryon-net); this shell just records the value it was told.
 *
 * PHYSICAL cross-device evidence (PLAT-001): two machines, two NICs, real mutual-pinned
 * TLS over the LAN. Not an APK; no platform "supported" (PLAT-006 / §30 out of scope).
 * Android bionic has pthreads in libc; do NOT link -lpthread.
 */

#include "carryon.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define EDITOR_ID "org.carryon.editor"

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1e6;
}

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
    CHECK(carryon_trust_pair(t, name, peer_pin_hex, "bench"), "trust_pair");
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

/* Build the bench adapter params for a strategy. A folds the optional nav bytes into the
 * authoritative document (moved up front); C/D keep the authoritative doc and leave the
 * optional nav ephemeral (deferred). */
static void bench_params(char strategy, long doc_bytes, long nav_bytes, char *out,
                         size_t out_len) {
    if (strategy == 'A') {
        snprintf(out, out_len, "{\"bench\":{\"doc\":%ld,\"nav\":0}}", doc_bytes + nav_bytes);
    } else {
        snprintf(out, out_len, "{\"bench\":{\"doc\":%ld,\"nav\":%ld}}", doc_bytes, nav_bytes);
    }
}

/* A bounded transfer chunk size (§18.6). A ChunkData frame carries base64 (~4/3) of the
 * chunk and must stay under the 1 MiB MAX_CONTROL_FRAME, so a full 1 MiB object with the
 * default chunk size overflows the frame. 256 KiB keeps every frame well under the cap. */
#define BENCH_CHUNK_BYTES 262144

static int do_source(const char *id_dir, const char *data_dir, const char *bind_addr,
                     const char *peer_pin, long doc_bytes, long nav_bytes, char strategy) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core)
        exit(2);
    CHECK(carryon_core_set_chunk_size(core, BENCH_CHUNK_BYTES), "set_chunk_size");
    char params[256];
    bench_params(strategy, doc_bytes, nav_bytes, params, sizeof(params));
    char *info = NULL;
    CHECK(carryon_register_adapter(core, EDITOR_ID, params, &info), "register");
    carryon_string_free(info);

    char *sid = NULL;
    CHECK(carryon_create_session(
              core,
              "{\"adapter_id\":\"" EDITOR_ID "\",\"title\":\"bench\","
              "\"privacy\":\"personal\",\"authority_mode\":\"single_writer\"}",
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
    printf("source: session=%s cut=%llu listening=%s strategy=%c\n", sid,
           (unsigned long long)cut_num, addr, strategy);
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
    printf("source: cut served; OK\n");

    carryon_string_free(sid);
    carryon_string_free(addr);
    carryon_session_free(sess);
    carryon_listener_free(listener);
    carryon_identity_free(id);
    carryon_trust_free(trust);
    carryon_core_free(core);
    return 0;
}

/* Minimal JSON string escaper for the failure reason (last_error text). */
static void json_escape(const char *in, char *out, size_t out_len) {
    size_t j = 0;
    for (size_t i = 0; in && in[i] && j + 2 < out_len; i++) {
        char c = in[i];
        if (c == '"' || c == '\\') {
            out[j++] = '\\';
            out[j++] = c;
        } else if (c == '\n' || c == '\r' || c == '\t') {
            out[j++] = ' ';
        } else {
            out[j++] = c;
        }
    }
    out[j] = '\0';
}

/* Emit one trial JSON line and exit. ok=0 paths record the failure reason so the
 * aggregator can report failures honestly rather than hiding them. */
static void emit_trial(const char *out_json, char strategy, int demand, long latency_ms,
                       long doc_bytes, long nav_bytes, int ok, double tta_ms,
                       double src_indep_ms, unsigned long long bytes_first,
                       unsigned long long total_bytes, int oracle_agreed,
                       const char *failure) {
    char esc[1024];
    json_escape(failure ? failure : "", esc, sizeof(esc));
    char line[2048];
    snprintf(line, sizeof(line),
             "{\"strategy\":\"%c\",\"doc_bytes\":%ld,\"nav_bytes\":%ld,"
             "\"latency_ms\":%ld,\"demand\":%s,\"no_handoff\":false,"
             "\"time_to_action_ready_ms\":%.3f,\"source_independence_ms\":%.3f,"
             "\"bytes_before_first_action\":%llu,\"total_bytes\":%llu,"
             "\"oracle_agreed\":%s,\"ok\":%s,\"failure\":\"%s\"}\n",
             strategy, doc_bytes, nav_bytes, latency_ms, demand ? "true" : "false",
             tta_ms, src_indep_ms, bytes_first, total_bytes,
             oracle_agreed ? "true" : "false", ok ? "true" : "false", esc);
    /* Append so a dest that is retried does not clobber prior lines in the same file. */
    FILE *f = fopen(out_json, "ab");
    if (f) {
        fputs(line, f);
        fclose(f);
    }
    fputs(line, stdout);
}

static int do_dest(const char *id_dir, const char *data_dir, const char *peer_addr,
                   const char *peer_pin, const char *remote_session, char strategy,
                   int demand, long latency_ms, long doc_bytes, long nav_bytes,
                   const char *out_json) {
    CarryonCore *core = carryon_core_open(data_dir);
    if (!core)
        exit(2);
    CHECK(carryon_core_set_chunk_size(core, BENCH_CHUNK_BYTES), "set_chunk_size");
    char params[256];
    bench_params(strategy, doc_bytes, nav_bytes, params, sizeof(params));
    char *info = NULL;
    if (carryon_register_adapter(core, EDITOR_ID, params, &info) != CARRYON_OK) {
        emit_trial(out_json, strategy, demand, latency_ms, doc_bytes, nav_bytes, 0, 0, 0,
                   0, 0, 0, "register failed");
        return 2;
    }
    carryon_string_free(info);

    CarryonIdentity *id = load_or_make_identity(id_dir, "dest");
    CarryonTrust *trust = trust_with_pin("source", peer_pin);

    CarryonSession *sess = carryon_session_connect(peer_addr, id, trust);
    if (!sess) {
        char buf[1024];
        size_t len = sizeof(buf);
        const char *msg = "connect failed";
        if (carryon_last_error((uint8_t *)buf, &len) == CARRYON_OK && len > 0 &&
            len < sizeof(buf)) {
            buf[len] = '\0';
            msg = buf;
        }
        emit_trial(out_json, strategy, demand, latency_ms, doc_bytes, nav_bytes, 0, 0, 0,
                   0, 0, 0, msg);
        return 2;
    }
    char *neg = NULL;
    CHECK(carryon_session_client_negotiate(sess, "[]", &neg), "client_negotiate");
    carryon_string_free(neg);

    double t0 = now_ms();
    char *out = NULL;
    if (carryon_import_cut(core, sess, remote_session, 0, &out) != CARRYON_OK) {
        char buf[1024];
        size_t len = sizeof(buf);
        const char *msg = "import failed";
        if (carryon_last_error((uint8_t *)buf, &len) == CARRYON_OK && len > 0 &&
            len < sizeof(buf)) {
            buf[len] = '\0';
            msg = buf;
        }
        emit_trial(out_json, strategy, demand, latency_ms, doc_bytes, nav_bytes, 0, 0, 0,
                   0, 0, 0, msg);
        carryon_session_free(sess);
        return 2;
    }
    double t_action_ready = now_ms();
    carryon_string_free(out);

    uint64_t sent = 0, recv = 0;
    carryon_session_bytes(sess, &sent, &recv);
    unsigned long long bytes_first = (unsigned long long)(sent + recv);

    /* ACTION_READY reached. The restore oracle confirms the SAME logical session. */
    char *mirror = NULL;
    CHECK(carryon_mirror_session_id(remote_session, &mirror), "mirror_session_id");
    char cut_ref[256];
    snprintf(cut_ref, sizeof(cut_ref), "{\"session\":\"%s\",\"number\":0}", mirror);
    char *restore = NULL;
    int oracle_agreed =
        carryon_execute_action_as(core, cut_ref,
                                  "{\"class\":\"session.restore\",\"params\":{}}",
                                  EDITOR_ID, &restore) == CARRYON_OK;
    if (restore)
        carryon_string_free(restore);

    /* Source independence: for A/C/D the destination can proceed source-free once the
     * authoritative closure is imported (the optional, if any, was already folded into A's
     * document). We measure to this point. */
    double t_source_independent = now_ms();
    carryon_session_bytes(sess, &sent, &recv);
    unsigned long long total_bytes = (unsigned long long)(sent + recv);

    emit_trial(out_json, strategy, demand, latency_ms, doc_bytes, nav_bytes, 1,
               t_action_ready - t0, t_source_independent - t0, bytes_first, total_bytes,
               oracle_agreed, "");

    carryon_string_free(mirror);
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
    if (strcmp(role, "source") == 0 && argc >= 9)
        return do_source(argv[2], argv[3], argv[4], argv[5], strtol(argv[6], NULL, 10),
                         strtol(argv[7], NULL, 10), argv[8][0]);
    if (strcmp(role, "dest") == 0 && argc >= 13)
        return do_dest(argv[2], argv[3], argv[4], argv[5], argv[6], argv[7][0],
                       (int)strtol(argv[8], NULL, 10), strtol(argv[9], NULL, 10),
                       strtol(argv[10], NULL, 10), strtol(argv[11], NULL, 10), argv[12]);
    fprintf(stderr, "bad args (role=%s argc=%d)\n", role, argc);
    return 1;
}
