/*
 * Carry-On C ABI — the full import→carry→relaunch lifecycle.
 *
 * Hand-written (cbindgen is unavailable offline). Kept in sync with the Rust
 * `#[no_mangle]` surface by tests/header_sync.rs. Append-only within an ABI major.
 *
 * Conventions:
 *  - Every fallible call returns int32_t: 0 = CARRYON_OK, negative = FFI-local
 *    error, positive = a core error (family_base + ordinal). Call
 *    carryon_last_error() for the message.
 *  - Functions returning a pointer return NULL on error (check carryon_last_error).
 *  - Out-strings (char**) are Rust-allocated; free each with carryon_string_free().
 *  - Handles are freed with their matching *_free(); input pointers are borrowed.
 *  - Out-buffer calls (cert/key/last_error) use the *len query protocol: pass the
 *    buffer capacity in *len; on success *len = bytes written, on
 *    CARRYON_ERR_BUFFER_TOO_SMALL *len = required size.
 *
 * Boundary: no entry point runs foreign code. carryon_register_adapter selects a
 * fixed compiled-in adapter by id + JSON params (data, never behavior).
 */
#ifndef CARRYON_H
#define CARRYON_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version. Check against carryon_abi_version() at load. */
#define CARRYON_ABI_MAJOR 1u
#define CARRYON_ABI_MINOR 0u

/* Result codes. */
#define CARRYON_OK                    0
#define CARRYON_ERR_PANIC            -1
#define CARRYON_ERR_NULL_ARG         -2
#define CARRYON_ERR_BAD_UTF8         -3
#define CARRYON_ERR_BAD_JSON         -4
#define CARRYON_ERR_BAD_HANDLE       -5
#define CARRYON_ERR_BUFFER_TOO_SMALL -6

/* Core error families: code = family_base + inner ordinal. */
#define CARRYON_E_ADAPTER_BASE   100
#define CARRYON_E_SCHEMA_BASE    200
#define CARRYON_E_OBJECT_BASE    300
#define CARRYON_E_TRANSFER_BASE  400
#define CARRYON_E_BUDGET_BASE    500
#define CARRYON_E_ACTION_BASE    600
#define CARRYON_E_AUTH_BASE      700
#define CARRYON_E_MATH_BASE      800
#define CARRYON_E_INTERNAL_BASE  900
#define CARRYON_E_PROTO_BASE    1000
#define CARRYON_E_PLATFORM_BASE 1100

/* Selected well-known codes (see errors.rs for the full table). */
#define CARRYON_E_ADAPTER_MISSING          (CARRYON_E_ADAPTER_BASE + 0)
#define CARRYON_E_OBJECT_DIGEST_MISMATCH   (CARRYON_E_OBJECT_BASE + 5)
#define CARRYON_E_TRANSFER_CONFLICTING     (CARRYON_E_TRANSFER_BASE + 3)
#define CARRYON_E_TRANSFER_RESUME_FAILURE  (CARRYON_E_TRANSFER_BASE + 5)
#define CARRYON_E_BUDGET_NETWORK           (CARRYON_E_BUDGET_BASE + 0)
#define CARRYON_E_AUTH_PERMISSION          (CARRYON_E_AUTH_BASE + 0)

/* Opaque handles. */
typedef struct CarryonCore CarryonCore;
typedef struct CarryonIdentity CarryonIdentity;
typedef struct CarryonTrust CarryonTrust;
typedef struct CarryonSession CarryonSession;
typedef struct CarryonListener CarryonListener;

/* ---- ABI / lifetime ---- */
void    carryon_abi_version(uint32_t *major, uint32_t *minor);
int32_t carryon_last_error(uint8_t *buf, size_t *len);
void    carryon_string_free(char *s);

/* ---- Core ---- */
CarryonCore *carryon_core_open(const char *data_dir);
void    carryon_core_free(CarryonCore *core);
int32_t carryon_core_recovery_report_json(CarryonCore *core, char **out_json);
int32_t carryon_core_set_chunk_size(CarryonCore *core, uint64_t bytes);
int32_t carryon_core_set_budget(CarryonCore *core, const char *budget_json);
int32_t carryon_core_set_foreground(CarryonCore *core, bool fg);
int32_t carryon_core_request_suspend(CarryonCore *core);

/* ---- Adapters / sessions ---- */
int32_t carryon_register_adapter(CarryonCore *core, const char *adapter_id,
                                 const char *params_json, char **out_info_json);
int32_t carryon_create_session(CarryonCore *core, const char *create_req_json,
                               char **out_session_id);

/* ---- Prepare / carry ---- */
int32_t carryon_create_cut(CarryonCore *core, const char *session_id, char **out_cut_json);
int32_t carryon_list_available_actions(CarryonCore *core, const char *cut_json,
                                       const char *classes_json, char **out_json);
int32_t carryon_execute_action_as(CarryonCore *core, const char *cut_json,
                                  const char *action_req_json, const char *adapter_id,
                                  char **out_result_json);
int32_t carryon_execute_action(CarryonCore *core, const char *cut_json,
                               const char *action_req_json, char **out_result_json);
int32_t carryon_has_object_hex(CarryonCore *core, const char *content_hash, bool *out_present);

/* ---- Net: identity / trust / pairing ---- */
CarryonIdentity *carryon_identity_generate(const char *name);
CarryonIdentity *carryon_identity_from_der(const char *name, const uint8_t *cert,
                                           size_t cert_len, const uint8_t *key, size_t key_len);
int32_t carryon_identity_cert_der(const CarryonIdentity *id, uint8_t *buf, size_t *len);
int32_t carryon_identity_key_der(const CarryonIdentity *id, uint8_t *buf, size_t *len); /* SECRET */
int32_t carryon_identity_pin_hex(const CarryonIdentity *id, char **out_hex);
void    carryon_identity_free(CarryonIdentity *id);

int32_t carryon_pair_devices(const CarryonIdentity *a, const CarryonIdentity *b,
                             const char *utc, char **out_json);
CarryonTrust *carryon_trust_from_json(const char *json);
int32_t carryon_trust_to_json(const CarryonTrust *trust, char **out_json);
int32_t carryon_trust_pair(CarryonTrust *trust, const char *name, const char *pin_hex,
                           const char *utc);
int32_t carryon_trust_is_trusted(const CarryonTrust *trust, const char *pin_hex, bool *out);
int32_t carryon_trust_revoke(CarryonTrust *trust, const char *pin_hex, bool *out);
void    carryon_trust_free(CarryonTrust *trust);

/* ---- Net: listener / session ---- */
CarryonListener *carryon_listener_bind(const char *addr);
int32_t carryon_listener_addr(const CarryonListener *listener, char **out_addr);
void    carryon_listener_free(CarryonListener *listener);
CarryonSession *carryon_session_connect(const char *addr, const CarryonIdentity *id,
                                        const CarryonTrust *trust);
CarryonSession *carryon_session_accept(const CarryonListener *listener,
                                       const CarryonIdentity *id, const CarryonTrust *trust);
int32_t carryon_session_client_negotiate(CarryonSession *session, const char *features_json,
                                         char **out_features_json);
int32_t carryon_session_server_negotiate(CarryonSession *session, const char *features_json,
                                         char **out_features_json);
void    carryon_session_free(CarryonSession *session);

/* ---- Transfer (handoff) ---- */
int32_t carryon_serve_cut(CarryonCore *core, CarryonSession *session);
int32_t carryon_import_cut(CarryonCore *core, CarryonSession *session,
                           const char *remote_session, uint64_t remote_cut,
                           char **out_result_json);
int32_t carryon_resume_import(CarryonCore *core, CarryonSession *session,
                              const char *resume_token_json, char **out_result_json);
int32_t carryon_read_object(const CarryonCore *core, const char *content_hash,
                            uint8_t *buf, size_t *len);
int32_t carryon_session_bytes(const CarryonSession *session, uint64_t *out_sent,
                              uint64_t *out_recv);

/* ---- Authority transfer (L4, spec 21.2/21.3) ---- */
int32_t carryon_serve_authority_transfer(CarryonCore *core, CarryonSession *session,
                                         const char *session_id, uint64_t cut_number,
                                         char **out_receipt_json);
int32_t carryon_request_authority_transfer(CarryonCore *core, CarryonSession *session,
                                           const char *mirror_session_id,
                                           const char *adapter_id, char **out_receipt_json);
int32_t carryon_recover_authority(CarryonCore *core, const char *session_id,
                                  uint64_t *out_epoch);
int32_t carryon_may_mutate(const CarryonCore *core, const char *session_id, bool *out);
int32_t carryon_mirror_session_id(const char *remote_session, char **out_mirror_id);

/* ---- Evidence ---- */
int32_t carryon_export_evidence(CarryonCore *core, const char *session_id, char **out_bundle_json);
int32_t carryon_export_evidence_with(CarryonCore *core, const char *session_id,
                                     const char *extra_metrics_json, char **out_bundle_json);
int32_t carryon_verify_evidence(const CarryonCore *core, const char *bundle_path,
                                char **out_report_json);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* CARRYON_H */
