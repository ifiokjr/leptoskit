# R8 keep rules for the Kotlin members the JNI library reaches by name. The
# CLI copies this file to .leptoskit/android/proguard-rules.pro on every
# Android build, so the rules always match the native code they guard. Keep
# each rule to the members a GetMethodID or FindClass call names: anything
# broader undoes release shrinking.

# bridge.cpp finds NativeRequest and PluginException by name and constructs
# them; plugins build against the rest of the bridge.
-keep class dev.leptoskit.bridge.** { *; }

# app.cpp looks these callbacks up on the receiver's class; the class itself
# may be renamed.
-keepclassmembers class dev.leptoskit.mobile.NativeServiceOwner {
    void wake();
    void close(long);
}
-keepclassmembers class dev.leptoskit.mobile.NativeRustAsync$Completion {
    void completed(long);
}
-keepclassmembers class dev.leptoskit.mobile.NativeTextMeasurer {
    float[] measure(byte[], float, int, float, boolean);
}
