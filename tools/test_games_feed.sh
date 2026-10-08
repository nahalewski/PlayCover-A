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
cat > "$work/Probe.kt" <<'EOF'
import java.io.File
import org.touchhle.android.RepoSources
fun main(args:Array<String>) {
    val feed=RepoSources.parseFeed(File(args[0]).readText())
    check(feed.apps.isNotEmpty() && feed.skipped==0)
    check(feed.apps.all { it.category==RepoSources.Category.GAMES })
    println("Parsed ${feed.apps.size} games; skipped ${feed.skipped}")
}
EOF
java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -classpath "$stdlib:$work/json.jar" -d "$work/probe.jar" "$work/Probe.kt" "$workspace/touchHLE-src/android/app/src/main/java/org/touchhle/android/RepoSources.kt"
java -cp "$work/probe.jar:$stdlib:$work/json.jar" ProbeKt "$workspace/repositories/games.json"
