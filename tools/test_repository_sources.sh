#!/bin/bash
# Compile the platform-independent helper against real JSON, without native builds.
set -euo pipefail
workspace=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cache="$HOME/.gradle/caches/modules-2/files-2.1"
compiler=$(find "$cache/org.jetbrains.kotlin/kotlin-compiler-embeddable/2.0.21" -name '*.jar' -print -quit)
stdlib=$(find "$cache/org.jetbrains.kotlin/kotlin-stdlib/2.0.21" -name '*.jar' -print -quit)
annotations=$(find "$cache/org.jetbrains/annotations/13.0" -name '*.jar' -print -quit)
reflect=$(find "$cache/org.jetbrains.kotlin/kotlin-reflect/1.6.10" -name '*.jar' -print -quit)
trove=$(find "$cache/org.jetbrains.intellij.deps/trove4j" -name '*.jar' -print -quit)
coroutines=$(find "$cache/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.6.4" -name '*.jar' -print -quit)
curl --fail --silent --show-error --location https://repo.maven.apache.org/maven2/org/json/json/20240303/json-20240303.jar -o "$work/json.jar"
curl --fail --silent --show-error --location https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar -o "$work/junit.jar"
curl --fail --silent --show-error --location https://repo.maven.apache.org/maven2/org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar -o "$work/hamcrest.jar"
cp="$stdlib:$annotations:$work/json.jar:$work/junit.jar:$work/hamcrest.jar"
java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler \
    -no-stdlib -no-reflect -jvm-target 11 -classpath "$cp" -d "$work/tests.jar" \
    "$workspace/touchHLE-src/android/app/src/main/java/org/touchhle/android/RepoSources.kt" \
    "$workspace/touchHLE-src/android/app/src/test/java/org/touchhle/android/RepoSourcesTest.kt"
java -cp "$work/tests.jar:$cp" org.junit.runner.JUnitCore org.touchhle.android.RepoSourcesTest
if [ "${1:-}" = "--actual" ]; then
    cat > "$work/ActualProbe.kt" <<'KOTLIN'
import org.touchhle.android.RepoSources
import java.io.File
fun main(args: Array<String>) {
    args.forEach { path -> RepoSources.validateIPA(File(path)); println("Validated IPA metadata: $path") }
    val feed = RepoSources.fetch(RepoSources.EXAMPLE_SOURCE, RepoSources.Transfer())
    println("Live repository: ${feed.name}; ${feed.apps.size} supported apps; ${feed.skipped} skipped")
}
KOTLIN
    java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler \
        -no-stdlib -no-reflect -jvm-target 11 -classpath "$cp:$work/tests.jar" -d "$work/actual.jar" "$work/ActualProbe.kt"
    java -cp "$work/actual.jar:$work/tests.jar:$cp" ActualProbeKt \
        /mnt/c/Users/Ben/Downloads/Photomath_8.42.0.ipa \
        /mnt/c/Users/Ben/Downloads/Terraria_4.5.0.ipa \
        /mnt/c/Users/Ben/Downloads/Dead_Cells_plus_v3.2.2.ipa \
        "$workspace/PokedexPlus-device.ipa"
fi
android_jar="$HOME/android-sdk/platforms/android-31/android.jar"
if [ -f "$android_jar" ]; then
    cat > "$work/MainActivity.kt" <<'KOTLIN'
package org.touchhle.android
class MainActivity : android.app.Activity() {
    companion object { const val EXTRA_APP_PATH = "app_path"; const val EXTRA_COMPAT = "compat_mode" }
}
KOTLIN
    java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler \
        -no-stdlib -no-reflect -jvm-target 11 -classpath "$cp:$android_jar" -d "$work/launcher.jar" \
        "$work/MainActivity.kt" \
        "$workspace/touchHLE-src/android/app/src/main/java/org/touchhle/android/IpaInfo.kt" \
        "$workspace/touchHLE-src/android/app/src/main/java/org/touchhle/android/RepoSources.kt" \
        "$workspace/touchHLE-src/android/app/src/main/java/org/touchhle/android/LauncherActivity.kt"
fi
