# ProGuard Rules for Reminisce Android App

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
-keep public class * extends com.bumptech.glide.GeneratedAppGlideModule
-keep class com.bumptech.glide.GeneratedAppGlideModuleImpl { *; }
-keep class org.openreminisce.app.util.MyAppGlideModule { *; }
-keep class org.openreminisce.app.util.ThumbnailAuthInterceptor { *; }
-keep class org.openreminisce.app.util.AuthenticatedHttpClient { *; }
-keepclassmembers class * extends okhttp3.Interceptor {
    public <init>(...);
}
-dontwarn com.bumptech.glide.**

# ExoPlayer / Media3
-keep class androidx.media3.common.** { *; }
-keep class androidx.media3.exoplayer.** { *; }
-keep class androidx.media3.ui.** { *; }
-dontwarn androidx.media3.**

# BouncyCastle (targeted keep for BLAKE3 hashing)
-keep class org.bouncycastle.crypto.digests.Blake3Digest { *; }
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
