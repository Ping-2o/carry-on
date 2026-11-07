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
# IMPORTED_NO_SONAME (device-run bug fix, 2026-11-07): the cargo cross-compiled
# engine cdylib carries NO SONAME (rustc does not pass -Wl,-soname for android
# targets), so a plain path link records the ABSOLUTE build-machine path in
# libcarryon_jni.so's DT_NEEDED — the CI APK then dlopen-fails on a real device
# (UnsatisfiedLinkError: library "/home/runner/work/.../libcarryon_ffi.so" not
# found; first device run crashed at the first FFI call). With
# IMPORTED_NO_SONAME the linker depends on the bare file name instead:
# DT_NEEDED = libcarryon_ffi.so, resolved by the Android linker from the APK's
# own native lib dir where the engine is packaged (jniLibs).
set_target_properties(carryon_ffi PROPERTIES
        IMPORTED_LOCATION ${CMAKE_CURRENT_SOURCE_DIR}/../jniLibs/${ANDROID_ABI}/libcarryon_ffi.so
        IMPORTED_NO_SONAME TRUE)

add_library(carryon_jni SHARED carryon_jni.c)

target_include_directories(carryon_jni PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/include)

target_link_libraries(carryon_jni carryon_ffi)

# ---- regression check (device-run bug, 2026-11-07) ----------------------------
# The bug class this guards against: an absolute or relative PATH baked into
# libcarryon_jni.so's DT_NEEDED for the engine (the cargo cdylib has no SONAME;
# without IMPORTED_NO_SONAME the linker records the build-machine path, and the
# APK dies on device at dlopen: UnsatisfiedLinkError). A custom target (not
# POST_BUILD, whose failures do not reliably fail the build) re-reads the just-
# linked shim with the NDK's llvm-readelf and FAILS the build unless
# DT_NEEDED entry for the engine is exactly "libcarryon_ffi.so" — bare file
# name, no slash, no build-machine prefix.
get_filename_component(_ndk_bin_dir ${CMAKE_C_COMPILER} DIRECTORY)
find_program(CARRYON_READELF llvm-readelf HINTS ${_ndk_bin_dir} NO_DEFAULT_PATH)
if(NOT CARRYON_READELF)
    message(FATAL_ERROR "llvm-readelf not found in the NDK toolchain (${_ndk_bin_dir})")
endif()
foreach(_cfg ${CMAKE_CONFIGURATION_TYPES} ${CMAKE_BUILD_TYPE})
    if(NOT _cfg)
        continue()
    endif()
    add_custom_target(verify_dt_needed_${_cfg} ALL
        COMMAND ${CMAKE_COMMAND}
            -DCARRYON_READELF=${CARRYON_READELF}
            -DSHIM=$<TARGET_FILE:carryon_jni>
            -DCFG=${_cfg}
            -P ${CMAKE_CURRENT_SOURCE_DIR}/verify_dt_needed.cmake
        DEPENDS carryon_jni
        VERBATIM
    )
endforeach()

# No libpthread on bionic (threads live in libc) and no C++ runtime needed for a
# pure-C shim (ANDROID_STL=none set in build.gradle.kts).
target_compile_options(carryon_jni PRIVATE -Wall -Wextra)
