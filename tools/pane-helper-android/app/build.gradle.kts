plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "tech.thothlab.pane.helper"
    compileSdk = 34

    defaultConfig {
        applicationId = "tech.thothlab.pane.helper"
        // Min 23 keeps us on the modern KeyChain semantics without losing
        // any device we'd reasonably support (Android 6 is from 2015).
        minSdk = 23
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"
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
