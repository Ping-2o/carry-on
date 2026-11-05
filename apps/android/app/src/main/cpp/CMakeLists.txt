# Carry-On for Android — native build (app-local JNI shim).
#
# The shim (carryon_jni.c) is pure translation over the REAL engine C ABI; the
# engine itself is the prebuilt cross-compiled libcarryon_ffi.so fetched into
# src/main/jniLibs/<ABI>/ by Scripts/fetch_engine_lib.sh (fetched, never
# committed). IMPORTED target: no engine sources are built here (plan.md hard
# rule — no Rust changes, no new FFI symbols).
#
# The vendored header at include/carryon.h is a byte-identical copy of
# crates/carryon-ffi/include/carryon.h; Scripts/fetch_engine_lib.sh re-syncs it
# and fails on drift (same discipline as the macOS app's sync_header.sh).
cmake_minimum_required(VERSION 3.22.1)
project(carryon_jni C)

add_library(carryon_ffi SHARED IMPORTED)
set_target_properties(carryon_ffi PROPERTIES IMPORTED_LOCATION
        ${CMAKE_CURRENT_SOURCE_DIR}/../jniLibs/${ANDROID_ABI}/libcarryon_ffi.so)

add_library(carryon_jni SHARED carryon_jni.c)

target_include_directories(carryon_jni PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/include)

target_link_libraries(carryon_jni carryon_ffi)

# No libpthread on bionic (threads live in libc) and no C++ runtime needed for a
# pure-C shim (ANDROID_STL=none set in build.gradle.kts).
target_compile_options(carryon_jni PRIVATE -Wall -Wextra)
