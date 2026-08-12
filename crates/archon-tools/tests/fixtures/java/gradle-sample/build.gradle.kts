// Gradle half of the Java fixture pair.
//
// Source lives one level up and is shared verbatim with maven-sample, so a
// difference in findings between the two is a difference between the build
// systems rather than between their inputs.
//
// Every analyser here is a build plugin resolved by Gradle itself. That is the
// whole reason archon's Java system dependencies are only JDK, Gradle and
// Maven: Checkstyle, PMD, SpotBugs and FindSecBugs need no system install and
// behave identically on every OS.

plugins {
    java
    checkstyle
    pmd
    id("com.github.spotbugs") version "6.5.10"
}

repositories {
    mavenCentral()
}

// Deliberately NOT a `toolchain { languageVersion = ... }` block. A toolchain
// pins an exact JDK, and Gradle fails outright with "no matching toolchains"
// when that JDK is absent rather than using the one it is already running on —
// so a fixture pinned to 21 breaks on a machine that has only 25. Setting the
// release instead compiles with whatever JDK is running, targeting 21 bytecode,
// which any JDK from 21 up can emit. This mirrors what the Maven half does with
// `maven.compiler.release`.
tasks.withType<JavaCompile>().configureEach {
    options.release = 21
}

sourceSets {
    main { java.setSrcDirs(listOf("../src")) }
    test { java.setSrcDirs(listOf("../test")) }
}

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.11.0")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
    spotbugsPlugins("com.h3xstream.findsecbugs:findsecbugs-plugin:1.13.0")
}

// Tool versions are pinned to the same releases the Maven fixture uses, so the
// two builds run identical rule catalogues over identical source. Without this
// the two halves would drift apart on their own defaults and any comparison
// between them would be meaningless.
checkstyle {
    toolVersion = "10.20.1"
    configFile = file("../config/checkstyle/checkstyle.xml")
    // The report file is what gets read. A tool that fails the build on its own
    // findings stops the later analysers from ever writing theirs.
    isIgnoreFailures = true
}

pmd {
    toolVersion = "7.3.0"
    // Gradle adds its own default rulesets unless this is cleared, which would
    // bury the two rules the fixture is built to trip under dozens of others.
    ruleSets = listOf()
    ruleSetFiles = files("../config/pmd/ruleset.xml")
    isIgnoreFailures = true
}

spotbugs {
    ignoreFailures = true
}

// SpotBugs defaults to an HTML report; the XML one is what carries the rule
// type and the cweid.
tasks.withType<com.github.spotbugs.snom.SpotBugsTask>().configureEach {
    reports.create("xml") { required = true }
}

tasks.test {
    useJUnitPlatform()
    // The fixture contains a deliberately failing test so the JUnit report has
    // something to parse; aborting here would leave it unwritten.
    ignoreFailures = true
}
