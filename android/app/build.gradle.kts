import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Firma de release: android/keystore.properties (local o generado en CI desde secretos).
val keystoreProps = rootProject.file("keystore.properties")

// Versión de la app: CI pasa -PappVersionName=<tag> -PappVersionCode=<run>.
val appVersionName = (findProperty("appVersionName") as String?) ?: "0.1.0"
val appVersionCode = (findProperty("appVersionCode") as String?)?.toIntOrNull() ?: 1

android {
    namespace = "dev.opencam.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.opencam.app"
        minSdk = 26
        targetSdk = 35
        versionCode = appVersionCode
        versionName = appVersionName
    }

    if (keystoreProps.exists()) {
        val props = Properties().apply { keystoreProps.inputStream().use { load(it) } }
        signingConfigs {
            create("release") {
                storeFile = rootProject.file(props.getProperty("storeFile"))
                storePassword = props.getProperty("storePassword")
                keyAlias = props.getProperty("keyAlias")
                keyPassword = props.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (keystoreProps.exists()) {
                signingConfig = signingConfigs.getByName("release")
            }
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
