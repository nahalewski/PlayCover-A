#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
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
cat > "$work/Bitmap.kt" <<'EOF'
package android.graphics
class Bitmap(val width:Int,val height:Int) { enum class Config { ARGB_8888 }; companion object { fun createBitmap(a:IntArray,w:Int,h:Int,c:Config)=Bitmap(w,h) } }
object BitmapFactory { class Options { var inJustDecodeBounds=false; var outWidth=0; var outHeight=0 }; fun decodeByteArray(d:ByteArray,o:Int,n:Int):Bitmap? = null; fun decodeByteArray(d:ByteArray,o:Int,n:Int,b:Options):Bitmap? = null }
EOF
cat > "$work/Log.kt" <<'EOF'
package android.util
object Log { fun w(tag:String,message:String):Int { System.err.println(message); return 0 }; fun w(tag:String,message:String,e:Throwable):Int { System.err.println(message); e.printStackTrace(); return 0 } }
EOF
cat > "$work/Probe.kt" <<'EOF'
import org.touchhle.android.IpaInfo
import java.io.File
fun main(args:Array<String>) { args.forEach { val i=IpaInfo.read(File(it)); println("$it: ${i?.displayName}, ${i?.icon?.width} x ${i?.icon?.height}") } }
EOF
sed 's/catch (e: Throwable) {/catch (e: Throwable) { e.printStackTrace();/' "$root/touchHLE-src/android/app/src/main/java/org/touchhle/android/IpaInfo.kt" > "$work/IpaInfo.kt"
java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -classpath "$stdlib:$HOME/android-sdk/platforms/android-31/android.jar" -d "$work/probe.jar" "$work/Bitmap.kt" "$work/Log.kt" "$work/Probe.kt" "$work/IpaInfo.kt"
java -cp "$work/probe.jar:$stdlib:$HOME/android-sdk/platforms/android-31/android.jar" ProbeKt "$root/pixel-fold-tests/Sonic1-icons.ipa" "$root/pixel-fold-tests/Sonic2-icons.ipa" "$root/pixel-fold-tests/PocketGod-icons.ipa"
curl --fail --silent --show-error https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar -o "$work/junit.jar"
curl --fail --silent --show-error https://repo.maven.apache.org/maven2/org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar -o "$work/hamcrest.jar"
java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -classpath "$stdlib:$HOME/android-sdk/platforms/android-31/android.jar:$work/junit.jar" -d "$work/tests.jar" "$work/Bitmap.kt" "$work/Log.kt" "$work/IpaInfo.kt" "$root/touchHLE-src/android/app/src/test/java/org/touchhle/android/IpaInfoTest.kt"
java -cp "$work/tests.jar:$stdlib:$HOME/android-sdk/platforms/android-31/android.jar:$work/junit.jar:$work/hamcrest.jar" org.junit.runner.JUnitCore org.touchhle.android.IpaInfoTest
