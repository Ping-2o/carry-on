/* czim.h — minimal C ABI over libzim (C++), for the Carry-On Kiwix adapter.
 *
 * Fail-closed contract: no error is ever swallowed into fabricated data.
 *   - pointer-returning functions return NULL on failure
 *   - string functions write at most buf_len-1 bytes + NUL and return the
 *     full needed length (>0) or <0 on error; callers re-call with a bigger
 *     buffer when the return equals/exceeds buf_len
 *   - czim_item_read returns bytes read (>=0) or -1 on error
 * No C++ exception ever crosses this boundary.
 */
#ifndef CZIM_H
#define CZIM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct czim_archive czim_archive;
typedef struct czim_item czim_item;

/* Open a ZIM archive by UTF-8 path (split zims: first part or logical path).
 * NULL on any failure (missing file, bad format, ...). */
czim_archive *czim_open(const char *utf8_path);

/* Close and free an archive (NULL is a no-op). */
void czim_close(czim_archive *arch);

/* --- archive-level --- */

/* ZIM uuid as the canonical string form; returns needed length or <0. */
int32_t czim_uuid(czim_archive *arch, char *buf, uint32_t buf_len);

/* Archive "Title" metadata; returns needed length, 0 if absent, or <0. */
int32_t czim_title(czim_archive *arch, char *buf, uint32_t buf_len);

/* Logical archive size in bytes (0 only if arch is NULL). */
uint64_t czim_filesize(czim_archive *arch);

/* Number of user entries (0 if arch is NULL). */
uint64_t czim_entry_count(czim_archive *arch);

/* Path of the main page entry (following redirects); <0 on error/absent. */
int32_t czim_main_path(czim_archive *arch, char *buf, uint32_t buf_len);

/* --- item-level --- */

/* Resolve an entry by path (redirects followed); NULL if not found.
 * The item keeps the (cached, decompressing) archive alive internally:
 * close the archive only after closing all items obtained from it. */
czim_item *czim_item_open(czim_archive *arch, const char *utf8_path);

void czim_item_close(czim_item *item);

/* Item data size in bytes (0 if item is NULL). */
uint64_t czim_item_size(czim_item *item);

/* Mimetype of the item; returns needed length or <0. */
int32_t czim_item_mimetype(czim_item *item, char *buf, uint32_t buf_len);

/* Path of the item (after redirect resolution); returns needed length or <0. */
int32_t czim_item_path(czim_item *item, char *buf, uint32_t buf_len);

/* Title of the item; returns needed length or <0. */
int32_t czim_item_title(czim_item *item, char *buf, uint32_t buf_len);

/* Read up to `len` bytes of the item's (decompressed) data at `offset`
 * into `buf`. Returns bytes read (>=0; 0 = at/after end) or -1 on error.
 * This is the chunked-transfer primitive: `Item::getData(offset, size)`. */
int64_t czim_item_read(czim_item *item, uint64_t offset, uint64_t len,
                       uint8_t *buf);

/* Direct access information (raw bytes inside the zim file, uncompressed
 * items only): 1 = available (path/offset filled), 0 = not available,
 * -1 = error. */
int32_t czim_item_direct_access(czim_item *item, char *path_buf,
                                uint32_t path_len, uint64_t *offset_out);

#ifdef __cplusplus
}
#endif

#endif /* CZIM_H */
