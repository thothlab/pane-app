plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "tech.thothlab.pane.helper"
    compileSdk = 34

    defaultConfig {
        applicationId = "tech.thothlab.pane.helper"
        // minSdk 29 = Android 10. Covers ~85% of active devices and is the
        // first version with foregroundServiceType in the manifest, which we
        // need for FOREGROUND_SERVICE_SPECIAL_USE on Android 14+.
        minSdk = 29
        targetSdk = 34
        versionCode = 2
        versionName = "2.0"
    }

    sourceSets {
        named("main") {
            java.srcDirs("src/main/kotlin")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Sign release with the debug keystore on purpose: this APK is
            // only ever installed via `adb install` from a trusted desktop
            // app on a USB-connected device with debugging enabled. There
            // is no Play distribution and no production signing key to
            // protect. Debug-signed APKs install silently over adb without
            // tripping signature mismatch on reinstall.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
}
