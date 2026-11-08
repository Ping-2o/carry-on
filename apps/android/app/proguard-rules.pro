# Carry-On for Android — R8 rules.
#
# The JNI shim and the engine binding now ship inside the :carryon-kit AAR,
# which carries its own consumer ProGuard rules (dev.carryon.kit.CarryOnFfi is
# kept there and merged into this app automatically). Nothing app-specific to
# keep for the FFI anymore — the SDK owns that contract.
#
# Coroutines debug metadata is safe to drop; everything else is default R8 for
# Compose + AndroidX (AGP ships the consumer rules).
