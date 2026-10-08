#!/bin/bash
set -euo pipefail
workspace=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cache="$HOME/.gradle/caches/modules-2/files-2.1"
findjar() { find "$cache/$1" -name '*.jar' -print -quit; }
compiler=$(findjar org.jetbrains.kotlin/kotlin-compiler-embeddable/2.0.21)
stdlib=$(findjar org.jetbrains.kotlin/kotlin-stdlib/2.0.21)
annotations=$(findjar org.jetbrains/annotations/13.0)
reflect=$(findjar org.jetbrains.kotlin/kotlin-reflect/1.6.10)
trove=$(findjar org.jetbrains.intellij.deps/trove4j)
coroutines=$(findjar org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.6.4)
curl --fail --silent --show-error https://repo.maven.apache.org/maven2/org/json/json/20240303/json-20240303.jar -o "$work/json.jar"
curl --fail --silent --show-error https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar -o "$work/junit.jar"
curl --fail --silent --show-error https://repo.maven.apache.org/maven2/org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar -o "$work/hamcrest.jar"
cat > "$work/LauncherActivity.kt" <<'EOF'
package org.touchhle.android
class LauncherActivity: android.app.Activity()
EOF
android_jar="$HOME/android-sdk/platforms/android-31/android.jar"
source="$workspace/touchHLE-src/android/app/src/main/java/org/touchhle/android"
test="$workspace/touchHLE-src/android/app/src/test/java/org/touchhle/android"
java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -classpath "$stdlib:$work/json.jar:$work/junit.jar:$android_jar" -d "$work/tests.jar" "$work/LauncherActivity.kt" "$source/RepoSources.kt" "$source/IpaDownloads.kt" "$source/IpaDownloadService.kt" "$test/IpaDownloadsTest.kt"
java -cp "$work/tests.jar:$stdlib:$work/json.jar:$work/junit.jar:$work/hamcrest.jar:$android_jar" org.junit.runner.JUnitCore org.touchhle.android.IpaDownloadsTest
