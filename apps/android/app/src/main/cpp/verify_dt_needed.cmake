# verify_dt_needed.cmake — regression guard for the 2026-11-07 device-run crash.
#
# The engine cdylib (libcarryon_ffi.so) is built by cargo WITHOUT a soname, so
# linking libcarryon_jni.so against it by path silently records that absolute
# build-machine path in DT_NEEDED — the APK then dlopen-fails on a real device:
#
#   java.lang.UnsatisfiedLinkError: dlopen failed: library
#   "/home/runner/work/carry-on/carry-on/apps/android/app/src/main/cpp/../jniLibs/
#    arm64-v8a/libcarryon_ffi.so" not found
#
# IMPORTED_NO_SONAME in CMakeLists.txt is the FIX; this script is the CHECK —
# it fails the build unless the just-linked shim depends on the engine by its
# bare file name exactly ("libcarryon_ffi.so"), never by a path.
#
# Invoked from CMakeLists.txt as a custom target (custom-target command
# failures DO fail the build, unlike POST_BUILD):
#   cmake -DCARRYON_READELF=<llvm-readelf> -DSHIM=<libcarryon_jni.so>
#         -DCFG=<config> -P verify_dt_needed.cmake

execute_process(
    COMMAND ${CARRYON_READELF} -d ${SHIM}
    OUTPUT_VARIABLE dyn
    OUTPUT_STRIP_TRAILING_WHITESPACE
    RESULT_VARIABLE rc
)
if(NOT rc EQUAL 0)
    message(FATAL_ERROR "verify_dt_needed (CFG=${CFG}): llvm-readelf failed on ${SHIM} (rc=${rc})")
endif()

# Every DT_NEEDED entry must be a bare file name — no '/', ever. This catches
# any path-shaped dependency (CI home dir, macOS build volume, relative
# ../jniLibs), regardless of which library it points at.
string(REPLACE ";" "" dyn_joined "${dyn}")
string(REGEX MATCHALL "Shared library: \\[[^]]*\\]" needed_entries "${dyn_joined}")

foreach(entry IN LISTS needed_entries)
    string(REGEX REPLACE "Shared library: \\[([^]]*)\\]" "\\1" lib "${entry}")
    if(lib MATCHES "/")
        message(FATAL_ERROR
            "verify_dt_needed (CFG=${CFG}): PATH-SHAPED DT_NEEDED '${lib}' in ${SHIM} — "
            "the APK would dlopen-fail on a device (UnsatisfiedLinkError). The engine "
            "prebuilt must be linked by bare file name (IMPORTED_NO_SONAME); see "
            "CMakeLists.txt and the 2026-11-07 device-run bug note.")
    endif()
endforeach()

if(NOT dyn_joined MATCHES "Shared library: \\[libcarryon_ffi\\.so\\]")
    message(FATAL_ERROR
        "verify_dt_needed (CFG=${CFG}): expected DT_NEEDED entry 'libcarryon_ffi.so' "
        "(the engine) missing from ${SHIM} — the shim links the engine; if the link "
        "inputs changed, update this guard deliberately.")
endif()

message(STATUS "verify_dt_needed (CFG=${CFG}): DT_NEEDED for the engine is the bare file name — OK")
