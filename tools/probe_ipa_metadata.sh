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
curl --fail --silent --show-error https://repo.maven.apache.org/maven2/net/sf/kxml/kxml2/2.3.0/kxml2-2.3.0.jar -o "$work/kxml.jar"
cat > "$work/Bitmap.kt" <<'EOF'
package android.graphics
class Bitmap(val width:Int,val height:Int) { enum class Config { ARGB_8888 }; companion object { fun createBitmap(a:IntArray,w:Int,h:Int,c:Config)=Bitmap(w,h) } }
object BitmapFactory { class Options { var inJustDecodeBounds=false; var outWidth=0; var outHeight=0 }; fun decodeByteArray(d:ByteArray,o:Int,n:Int):Bitmap? = null; fun decodeByteArray(d:ByteArray,o:Int,n:Int,b:Options):Bitmap? = null }
EOF
cat > "$work/Xml.kt" <<'EOF'
package android.util
object Xml { fun newPullParser():org.xmlpull.v1.XmlPullParser = org.kxml2.io.KXmlParser() }
object Log { fun w(tag:String,message:String):Int = 0; fun w(tag:String,message:String,e:Throwable):Int { e.printStackTrace(); return 0 } }
EOF
cat > "$work/Probe.kt" <<'EOF'
import org.touchhle.android.IpaInfo
import java.io.File
fun main(args:Array<String>) { args.forEach { path ->
 val info=IpaInfo.read(File(path))
 println("$path\t${info?.displayName}\t${info?.minimumIosVersion}\t${info?.is64Bit}")
} }
EOF
java -cp "$compiler:$stdlib:$annotations:$reflect:$trove:$coroutines" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -classpath "$stdlib:$work/kxml.jar:$HOME/android-sdk/platforms/android-31/android.jar" -d "$work/probe.jar" "$work/Bitmap.kt" "$work/Xml.kt" "$work/Probe.kt" "$root/touchHLE-src/android/app/src/main/java/org/touchhle/android/IpaInfo.kt" >/dev/null
java -cp "$work/probe.jar:$stdlib:$work/kxml.jar:$HOME/android-sdk/platforms/android-31/android.jar" ProbeKt "$@"
