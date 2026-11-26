import java.io.ByteArrayOutputStream

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The commit built, for the About line (and comparing with Actions builds).
val commit: String = System.getenv("GITHUB_SHA") ?: runCatching {
    val out = ByteArrayOutputStream()
    val p = ProcessBuilder("git", "rev-parse", "HEAD").redirectErrorStream(true).start()
    p.inputStream.copyTo(out)
    if (p.waitFor() == 0) out.toString().trim() else ""
}.getOrDefault("")

// The version: YOINK_VERSION_NAME when CI sets it (a v* tag's number), otherwise the
// crate's version and the commit, so every APK says what it is in its name and in Options.
val cargoVersion = Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(rootProject.file("../Cargo.toml").readText())!!.groupValues[1]
val appVersion = System.getenv("YOINK_VERSION_NAME")?.takeIf { it.isNotBlank() }
    ?: listOf(cargoVersion, commit.take(7)).filter { it.isNotEmpty() }.joinToString("-")

// A release keystore from the environment (CI secrets). Without one, every
// build (debug, release, local or CI) is signed with the dev keystore kept
// in the repository, so each build installs over the last one instead of
// Android refusing it for another signature (as it would with each
// machine's own throwaway debug key).
val keystorePath: String? = System.getenv("YOINK_KEYSTORE")?.takeIf { it.isNotBlank() && file(it).exists() }
val devKeystore = rootProject.file("yoink-dev.keystore")

android {
    namespace = "io.github.singularity098.yoink"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.singularity098.yoink"
        minSdk = 26
        targetSdk = 36
        // CI numbers builds by run so every Actions build is an update.
        versionCode = System.getenv("GITHUB_RUN_NUMBER")?.toIntOrNull() ?: 1
        versionName = appVersion
        buildConfigField("String", "COMMIT", "\"$commit\"")
    }

    signingConfigs {
        create("dev") {
            storeFile = devKeystore
            storePassword = "yoink-dev"
            keyAlias = "yoink-dev"
            keyPassword = "yoink-dev"
        }
        if (keystorePath != null) {
            create("release") {
                storeFile = file(keystorePath)
                storePassword = System.getenv("YOINK_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("YOINK_KEY_ALIAS")
                keyPassword = System.getenv("YOINK_KEY_PASSWORD") ?: System.getenv("YOINK_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        debug {
            signingConfig = signingConfigs.getByName("dev")
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("dev")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // Yoink-<version>.apk (and Yoink-<version>-debug.apk) instead of app-release.apk.
    applicationVariants.all {
        val variant = this
        val type = if (variant.buildType.name == "release") "" else "-${variant.buildType.name}"
        outputs.all {
            (this as com.android.build.gradle.internal.api.BaseVariantOutputImpl).outputFileName =
                "Yoink-${variant.versionName}$type.apk"
        }
    }

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    val bom = platform("androidx.compose:compose-bom:2025.12.01")
    implementation(bom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")

    testImplementation("junit:junit:4.13.2")
    // The real org.json for unit tests (Android's is a stub on the JVM).
    testImplementation("org.json:json:20250517")
}
