# Carry-On for Android — R8 rules.
#
# The JNI shim resolves `external fun`s on com.carryon.app.ffi.CarryOnFfi by name
# from native code (Java_com_carryon_app_ffi_CarryOnFfi_*). Never rename or strip
# that class, and keep the fields used as handles.
-keep class com.carryon.app.ffi.** { *; }

# Coroutines debug metadata is safe to drop; everything else is default R8 for
# Compose + AndroidX (AGP ships the consumer rules).
