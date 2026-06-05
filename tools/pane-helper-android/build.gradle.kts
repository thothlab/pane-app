// Top-level. Plugins applied per-module to avoid leaking into `:buildSrc`
// (we don't have one, but keeps things tidy if we add it later).
plugins {
    id("com.android.application") version "8.5.0" apply false
    id("org.jetbrains.kotlin.android") version "1.9.24" apply false
}
