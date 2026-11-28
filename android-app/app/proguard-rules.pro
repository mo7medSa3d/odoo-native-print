# Strip all android.util.Log calls in release builds
-assumenosideeffects class android.util.Log {
    public static boolean isLoggable(java.lang.String, int);
    public static int v(...);
    public static int d(...);
    public static int i(...);
    public static int w(...);
    public static int e(...);
    public static int wtf(...);
}

# Narrowest keep rules: only @JavascriptInterface methods and WebMessageListener bridge
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}
-keep class com.company.odoonativeprint.bridge.BridgeDispatcher {
    *;
}
-keep class com.company.odoonativeprint.bridge.AndroidWebBridgeListener {
    *;
}
