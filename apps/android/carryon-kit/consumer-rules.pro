# Carry-On Kit — consumer ProGuard/R8 rules.
#
# The bridge class and its native method names are looked up by the JVM at
# System.loadLibrary + first call; renaming or stripping them yields an
# UnsatisfiedLinkError at runtime. Keep the bridge intact and keep the names
# of every class that declares native methods.

-keep class dev.carryon.kit.CarryOnFfi { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}

# The typed wrappers are ordinary Kotlin; consumers may shrink them freely —
# but the public API surface stays (SDK consumers reference it).
-keep public class dev.carryon.kit.Core { public *; }
-keep public class dev.carryon.kit.Identity { public *; }
-keep public class dev.carryon.kit.Trust { public *; }
-keep public class dev.carryon.kit.Listener { public *; }
-keep public class dev.carryon.kit.Session { public *; }
-keep public class dev.carryon.kit.CarryOnKit { public *; }
-keep class dev.carryon.kit.Err { *; }
-keep class dev.carryon.kit.CoreException { *; }
