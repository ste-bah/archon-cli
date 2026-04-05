plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "1.9.22"
    id("org.jetbrains.intellij") version "1.17.0"
}

group = "dev.archon"
version = "0.1.0"

intellij {
    version.set("2024.1")
    type.set("IC")
    plugins.set(emptyList<String>())
}

repositories { mavenCentral() }

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlin:kotlin-test-junit:1.9.22")
}

tasks.test { useJUnit() }
tasks.buildSearchableOptions { enabled = false }
