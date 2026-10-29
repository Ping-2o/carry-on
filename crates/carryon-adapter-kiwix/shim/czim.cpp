/* czim.cpp — implementation of the minimal C ABI over libzim.
 *
 * Every entry catches all exceptions and fails closed (NULL / <0 / -1).
 * See czim.h for the contract. Labels: this shim is IMPLEMENTED on the
 * macOS host (see docs/kiwix-bridge/BUILD-EVIDENCE.md); cross builds are
 * recorded there per target.
 */
#include "czim.h"

#include <zim/archive.h>
#include <zim/entry.h>
#include <zim/item.h>

#include <cstring>
#include <memory>
#include <new>
#include <string>

namespace {

/* Writes s into (buf, buf_len) NUL-terminated; returns the full length of s
 * (so callers can detect truncation: ret >= buf_len means re-call). */
int32_t write_str(char *buf, uint32_t buf_len, const std::string &s) {
  if (buf == nullptr || buf_len == 0) {
    return static_cast<int32_t>(s.size()) + 1;
  }
  const uint32_t n = s.size();
  if (n >= buf_len) {
    std::memcpy(buf, s.data(), buf_len - 1);
    buf[buf_len - 1] = '\0';
  } else {
    std::memcpy(buf, s.data(), n);
    buf[n] = '\0';
  }
  return static_cast<int32_t>(n) + 1;
}

} // namespace

struct czim_archive {
  std::shared_ptr<zim::Archive> arch;
};

struct czim_item {
  /* Keep a shared_ptr to the archive so a stale item cannot dangle even if
   * the caller closes the archive first. */
  std::shared_ptr<zim::Archive> arch;
  zim::Item item;
};

extern "C" {

czim_archive *czim_open(const char *utf8_path) {
  if (utf8_path == nullptr) {
    return nullptr;
  }
  try {
    auto arch = std::make_shared<zim::Archive>(std::string(utf8_path));
    auto out = new (std::nothrow) czim_archive{std::move(arch)};
    return out;
  } catch (...) {
    return nullptr;
  }
}

czim_archive *czim_open_noxapian(const char *utf8_path) {
  if (utf8_path == nullptr) {
    return nullptr;
  }
  try {
    /* Progressive open for partially-materialized archives:
     * - no eager Xapian preload (lazy; fails closed while sparse);
     * - preloadDirentRanges(0): libzim's DEFAULT OpenConfig preloads
     *   DIRENT_LOOKUP_CACHE_SIZE (1024) dirent ranges at open, which
     *   touches ~1025 dirents spread over the whole dirent region — the
     *   whole region would have to be materialized. With 0, libzim uses
     *   the lazy DirentLookup binary search (src/dirent_lookup.h), which
     *   reads only the O(log n) dirents each lookup actually probes. */
    auto arch = std::make_shared<zim::Archive>(
        std::string(utf8_path),
        zim::OpenConfig().preloadDirentRanges(0).preloadXapianDb(false));
    auto out = new (std::nothrow) czim_archive{std::move(arch)};
    return out;
  } catch (...) {
    return nullptr;
  }
}

void czim_close(czim_archive *arch) { delete arch; }

int32_t czim_uuid(czim_archive *arch, char *buf, uint32_t buf_len) {
  if (arch == nullptr) {
    return -1;
  }
  try {
    const std::string uuid(static_cast<std::string>(arch->arch->getUuid()));
    return write_str(buf, buf_len, uuid);
  } catch (...) {
    return -1;
  }
}

int32_t czim_title(czim_archive *arch, char *buf, uint32_t buf_len) {
  if (arch == nullptr) {
    return -1;
  }
  try {
    const std::string title = arch->arch->getMetadata("Title");
    return write_str(buf, buf_len, title);
  } catch (...) {
    return -1;
  }
}

uint64_t czim_filesize(czim_archive *arch) {
  if (arch == nullptr) {
    return 0;
  }
  try {
    return arch->arch->getFilesize();
  } catch (...) {
    return 0;
  }
}

uint64_t czim_entry_count(czim_archive *arch) {
  if (arch == nullptr) {
    return 0;
  }
  try {
    return arch->arch->getEntryCount();
  } catch (...) {
    return 0;
  }
}

int32_t czim_main_path(czim_archive *arch, char *buf, uint32_t buf_len) {
  if (arch == nullptr) {
    return -1;
  }
  try {
    /* getItem(true) resolves the whole redirect chain. */
    const zim::Item main = arch->arch->getMainEntry().getItem(true);
    return write_str(buf, buf_len, main.getPath());
  } catch (...) {
    return -1;
  }
}

czim_item *czim_item_open(czim_archive *arch, const char *utf8_path) {
  if (arch == nullptr || utf8_path == nullptr) {
    return nullptr;
  }
  try {
    /* Compatibility layer: getEntryByPath handles both namespace schemes;
     * getItem(true) resolves the whole redirect chain. */
    const zim::Item item =
        arch->arch->getEntryByPath(std::string(utf8_path)).getItem(true);
    auto out = new (std::nothrow)
        czim_item{arch->arch, std::move(item)};
    return out;
  } catch (...) {
    return nullptr;
  }
}

void czim_item_close(czim_item *item) { delete item; }

uint64_t czim_item_size(czim_item *item) {
  if (item == nullptr) {
    return 0;
  }
  try {
    return item->item.getSize();
  } catch (...) {
    return 0;
  }
}

int32_t czim_item_mimetype(czim_item *item, char *buf, uint32_t buf_len) {
  if (item == nullptr) {
    return -1;
  }
  try {
    return write_str(buf, buf_len, item->item.getMimetype());
  } catch (...) {
    return -1;
  }
}

int32_t czim_item_path(czim_item *item, char *buf, uint32_t buf_len) {
  if (item == nullptr) {
    return -1;
  }
  try {
    return write_str(buf, buf_len, item->item.getPath());
  } catch (...) {
    return -1;
  }
}

int32_t czim_item_title(czim_item *item, char *buf, uint32_t buf_len) {
  if (item == nullptr) {
    return -1;
  }
  try {
    return write_str(buf, buf_len, item->item.getTitle());
  } catch (...) {
    return -1;
  }
}

int64_t czim_item_read(czim_item *item, uint64_t offset, uint64_t len,
                      uint8_t *buf) {
  if (item == nullptr || buf == nullptr) {
    return -1;
  }
  if (len == 0) {
    return 0;
  }
  try {
    /* Chunked transfer primitive: the item-level offset read. */
    const uint64_t size = item->item.getSize();
    if (offset >= size) {
      return 0;
    }
    const uint64_t avail = size - offset;
    const uint64_t n = len < avail ? len : avail;
    const zim::Blob blob = item->item.getData(offset, n);
    const uint64_t got = blob.size();
    if (got > n) { /* defensive: libzim should never over-read */
      return -1;
    }
    std::memcpy(buf, blob.data(), got);
    return static_cast<int64_t>(got);
  } catch (...) {
    return -1;
  }
}

int32_t czim_item_direct_access(czim_item *item, char *path_buf,
                               uint32_t path_len, uint64_t *offset_out) {
  if (item == nullptr || offset_out == nullptr) {
    return -1;
  }
  try {
    const zim::ItemDataDirectAccessInfo info =
        item->item.getDirectAccessInformation();
    if (info.filename.empty() && info.offset == 0) {
      return 0;
    }
    *offset_out = info.offset;
    if (path_buf != nullptr && path_len > 0) {
      const uint32_t n =
          info.filename.size() < path_len - 1
              ? static_cast<uint32_t>(info.filename.size())
              : path_len - 1;
      std::memcpy(path_buf, info.filename.data(), n);
      path_buf[n] = '\0';
    }
    return 1;
  } catch (...) {
    return -1;
  }
}

} /* extern "C" */
