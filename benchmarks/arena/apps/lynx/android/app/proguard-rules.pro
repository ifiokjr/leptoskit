# The Lynx engine's keep rules, from lynxjs.org "Integrate with Existing Apps"
# (Android). Native modules are LynxModule subclasses, which these keep.
# LYNX START
# use @Keep to annotate retained classes.
-dontwarn android.support.annotation.Keep
-keep @android.support.annotation.Keep class **
-keep @android.support.annotation.Keep class ** {
    @android.support.annotation.Keep <fields>;
    @android.support.annotation.Keep <methods>;
}
-dontwarn androidx.annotation.Keep
-keep @androidx.annotation.Keep class **
-keep @androidx.annotation.Keep class ** {
    @androidx.annotation.Keep <fields>;
    @androidx.annotation.Keep <methods>;
}
# native method call
-keepclasseswithmembers,includedescriptorclasses class * {
    native <methods>;
}
-keepclasseswithmembers class * {
    @com.lynx.tasm.base.CalledByNative <methods>;
}

# to customize a module, you need to keep the class name and the method annotated as LynxMethod.
-keepclasseswithmembers class * {
    @com.lynx.jsbridge.LynxMethod <methods>;
}

-keepclassmembers class *  {
    @com.lynx.tasm.behavior.LynxProp <methods>;
    @com.lynx.tasm.behavior.LynxPropGroup <methods>;
    @com.lynx.tasm.behavior.LynxUIMethod <methods>;
}

-keepclassmembers class com.lynx.tasm.behavior.ui.UIGroup {
    public boolean needCustomLayout();
}

# in case R8 compiler may remove mLoader in bytecode.
# as mLoader is not used in java and passed as a WeakRef in JNI.
-keepclassmembers class com.lynx.tasm.LynxTemplateRender {
    private com.lynx.tasm.core.resource.LynxResourceLoader mLoader;
    private com.lynx.tasm.core.resource.LynxResourceLoader mResourceLoader;
}

# the automatically generated setter classes use the class names of LynxBaseUI and ShadowNode and their subclasses.
-keep class com.lynx.tasm.behavior.ui.LynxBaseUI
-keep class com.lynx.tasm.behavior.shadow.ShadowNode
-keep class com.lynx.jsbridge.LynxModule { *; }
-keep class * extends com.lynx.tasm.behavior.ui.LynxBaseUI
-keep class * extends com.lynx.tasm.behavior.shadow.ShadowNode
-keep class * extends com.lynx.jsbridge.LynxModule { *; }
-keep class * extends com.lynx.jsbridge.LynxContextModule
-keep class * implements com.lynx.tasm.behavior.utils.Settable
-keep class * implements com.lynx.tasm.behavior.utils.LynxUISetter
-keep class * implements com.lynx.tasm.behavior.utils.LynxUIMethodInvoker
-keep class com.lynx.tasm.rendernode.compat.**{
    *;
}
-keep class com.lynx.tasm.rendernode.compat.RenderNodeFactory{
    *;
}
# LYNX END

# Optional dependencies the engine and the XElement registries reference but
# this host does not ship: Fresco (image loading), Gson (debug descriptions),
# and the XElement modules other than input. Nothing on the arena's paths
# reaches them.
-dontwarn com.facebook.common.**
-dontwarn com.facebook.datasource.**
-dontwarn com.facebook.drawee.**
-dontwarn com.facebook.imagepipeline.**
-dontwarn com.google.gson.**
-dontwarn com.lynx.markdown.**
-dontwarn com.lynx.animax.**
-dontwarn com.lynx.xelement.blur.**
-dontwarn com.lynx.xelement.markdown.**
-dontwarn com.lynx.xelement.overlay.**
-dontwarn com.lynx.xelement.refresh.**
-dontwarn com.lynx.xelement.scroll.**
-dontwarn com.lynx.xelement.svg.**
-dontwarn com.lynx.xelement.video.**
-dontwarn com.lynx.xelement.viewpager.**
-dontwarn com.lynx.xelement.webview.**

# The engine instantiates the generated `$$PropsSetter` and `$$MethodInvoker`
# classes by reflection. The published rules keep the classes but not their
# constructors, and without them R8 breaks property updates and UI methods
# (for example `<list>`'s scrollToPosition) in release builds.
-keep class **$$PropsSetter { <init>(...); *; }
-keep class **$$MethodInvoker { <init>(...); *; }
