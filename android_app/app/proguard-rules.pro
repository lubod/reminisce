# ProGuard Rules for Reminisce Android App

# Keep data models used by Gson serialization/deserialization
-keepclassmembers class * {
    @com.google.gson.annotations.SerializedName <fields>;
}
-keep class org.openreminisce.app.model.** { *; }

# OkHttp
-keepattributes Signature
-keepattributes AnnotationDefault
-keepclassmembers class * extends okhttp3.EventListener {
    public <init>(...);
}
-dontwarn okhttp3.**
-dontwarn okio.**
-dontwarn javax.annotation.**
-dontwarn org.conscrypt.**

# Glide
-keep public class * extends com.bumptech.glide.module.AppGlideModule
-keep class * implements com.bumptech.glide.module.GlideModule
-keepclassmembers class * implements com.bumptech.glide.module.GlideModule {
    public <init>(...);
}
-keepclassmembers class com.bumptech.glide.integration.okhttp3.OkHttpLibraryGlideModule {
    public <init>(...);
}
-dontwarn com.bumptech.glide.**

# ExoPlayer / Media3
-keep class androidx.media3.common.** { *; }
-keep class androidx.media3.exoplayer.** { *; }
-keep class androidx.media3.ui.** { *; }
-dontwarn androidx.media3.**

# BouncyCastle (for BLAKE3 hashing)
-keep class org.bouncycastle.** { *; }
-dontwarn org.bouncycastle.**

# AndroidX Security / Crypto
-keep class androidx.security.crypto.** { *; }

# WorkManager
-keep class * extends androidx.work.Worker {
    public <init>(android.content.Context, androidx.work.WorkerParameters);
}
-keep class * extends androidx.work.ListenableWorker {
    public <init>(android.content.Context, androidx.work.WorkerParameters);
}
