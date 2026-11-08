/*
 * shim.c — the CCarryon target carries the vendored carryon.h only (the engine
 * itself is the Rust dylib). This empty translation unit just validates that the
 * header compiles standalone in the target.
 */
#include "carryon.h"

int carryon_ccarryon_shim(void) { return 0; }
