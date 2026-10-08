package org.touchhle.android

import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.net.URLDecoder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.zip.ZipFile
import javax.xml.parsers.DocumentBuilderFactory

/** Plain source metadata; URLs and archives are checked before entering the library. */
object RepoSources {
    const val EXAMPLE_SOURCE = "https://ipa.cypwn.xyz/cypwn_altstore.json"
    internal const val FEED_LIMIT = 32 * 1024 * 1024
    private const val IPA_LIMIT = 2L * 1024 * 1024 * 1024
    data class Source(val name: String, val url: String)
    enum class Category(val label: String) { GAMES("Games"), EMULATORS("Emulators"), OTHER("Other") }
    data class App(val name: String, val identifier: String, val developer: String,
                   val version: String, val downloadURL: String, val iconURL: String? = null,
                   val category: Category = Category.OTHER)
    data class Feed(val name: String, val apps: List<App>, val skipped: Int)

    class Transfer {
        val cancelled = AtomicBoolean(false)
        @Volatile private var connection: HttpURLConnection? = null
        fun cancel() { cancelled.set(true); connection?.disconnect() }
        internal fun attach(value: HttpURLConnection?) {
            connection = value
            if (cancelled.get()) { value?.disconnect(); throw IOException("Cancelled") }
        }
        internal fun check() { if (cancelled.get()) throw IOException("Cancelled") }
    }

    fun httpsURL(value: String): String {
        require(value.length in 1..8192 && value.none { it.isISOControl() }) { "Invalid URL" }
        val uri = URI(value)
        require(uri.scheme.equals("https", true) && !uri.host.isNullOrEmpty() &&
            uri.rawUserInfo == null && uri.rawFragment == null) { "Use an HTTPS URL without credentials or a fragment" }
        return uri.toASCIIString()
    }

    fun sourceURL(input: String): String {
        val value = input.trim()
        if (value.startsWith("https:", true)) return httpsURL(value)
        val uri = URI(value)
        require(uri.scheme.equals("altstore", true) && uri.host.equals("source", true) &&
            uri.rawUserInfo == null && uri.port == -1 && uri.rawFragment == null &&
            (uri.rawPath.isNullOrEmpty() || uri.rawPath == "/")) { "Use an HTTPS source or altstore://source?url= link" }
        val values = uri.rawQuery.orEmpty().split('&').map { it.split('=', limit = 2) }
            .filter { it.size == 2 && it[0] == "url" }
        require(values.size == 1) { "The source link must contain one url parameter" }
        // URI query decoding preserves literal plus; form decoding would corrupt it.
        return httpsURL(URLDecoder.decode(values.single()[1].replace("+", "%2B"), "UTF-8"))
    }

    private fun text(value: String, limit: Int = 256) = value.filter { !it.isISOControl() }.take(limit)

    private fun category(app: JSONObject, name: String, identifier: String): Category {
        val explicit = ArrayList<String>()
        listOf("category", "categories", "genre", "genres", "primaryGenreName", "primaryGenreId").forEach { key ->
            when (val value = app.opt(key)) {
                is String -> explicit.add(text(value, 128).lowercase(java.util.Locale.ROOT))
                is Number -> explicit.add(value.toString())
                is org.json.JSONArray -> for (i in 0 until minOf(value.length(), 32)) {
                    (value.opt(i) as? String)?.let { explicit.add(text(it, 128).lowercase(java.util.Locale.ROOT)) }
                }
            }
        }
        if (explicit.any { it in setOf("emulator", "emulators", "emulation") }) return Category.EMULATORS
        if (explicit.any { it in setOf("game", "games", "gaming", "6014") }) return Category.GAMES
        // A source's explicit non-game classification takes precedence over heuristics.
        if (explicit.isNotEmpty()) return Category.OTHER
        val title = name.lowercase(java.util.Locale.ROOT).replace('_', ' ')
        val id = identifier.lowercase(java.util.Locale.ROOT)
        val description = text(app.optString("localizedDescription", app.optString("description")), 8192)
            .lowercase(java.util.Locale.ROOT)
        val words = "$title $id"
        if (Regex("\\b(emulator|emulation|retroarch|ppsspp|dolphinios|flycast|provenance|inds|desmume|gba4ios)\\b").containsMatchIn(words) ||
            Regex("^(?:delta|ignited|folium)(?:$| [v0-9])").containsMatchIn(title) || id == "com.rileytestut.delta" ||
            Regex("\\b(?:console|game|nintendo|playstation|psp|nds|gba) emulator\\b").containsMatchIn(description)) return Category.EMULATORS
        if (Regex("\\b(minecraft|terraria|stardew|sonic|angry birds|pocket god|dead cells|geometry dash|subway surfers|temple run|deltarune|balatro|fortnite|little nightmares|hollowknight|true skate)\\b").containsMatchIn(title) ||
            Regex("\\b(?:a|an|the) (?:(?:classic|mobile|arcade|puzzle|racing|action|adventure|platform|role-playing|video|strategy) )+(?:game|platformer)\\b").containsMatchIn(description) ||
            Regex("\\b(?:a|an) (?:platformer|roguelike|rpg)\\b").containsMatchIn(description)) return Category.GAMES
        return Category.OTHER
    }

    fun parseFeed(json: String): Feed {
        // Avoid allocating another complete feed just to measure UTF-8 bytes.
        require(feedFits(json, FEED_LIMIT)) { "Source is too large" }
        val root = JSONObject(json)
        val name = text(root.getString("name")).also { require(it.isNotBlank()) { "Source name is missing" } }
        val array = root.getJSONArray("apps")
        require(array.length() <= 20000) { "Source contains too many apps" }
        val apps = ArrayList<App>()
        var skipped = 0
        val identifiers = HashSet<String>()
        for (index in 0 until array.length()) {
            try {
                val app = array.getJSONObject(index)
                val latest = if (app.has("versions")) app.getJSONArray("versions").getJSONObject(0) else app
                val identifier = text(app.getString("bundleIdentifier"))
                val appName = text(app.getString("name"))
                require(identifier.isNotBlank() && appName.isNotBlank())
                val download = httpsURL(latest.getString("downloadURL"))
                require(identifiers.add(identifier))
                apps.add(App(appName, identifier, text(app.optString("developerName")),
                    text(latest.getString("version")), download,
                    app.optString("iconURL").takeIf { it.isNotBlank() }?.let { runCatching { httpsURL(it) }.getOrNull() },
                    category(app, appName, identifier)))
            } catch (_: Exception) { skipped++ }
        }
        return Feed(name, apps, skipped)
    }

    internal fun feedFits(value: String, limit: Int): Boolean {
        var bytes = 0L
        var index = 0
        while (index < value.length) {
            val ch = value[index++]
            bytes += when {
                ch.code < 0x80 -> 1
                ch.code < 0x800 -> 2
                ch.isHighSurrogate() && index < value.length && value[index].isLowSurrogate() -> { index++; 4 }
                ch.isSurrogate() -> 1 // JVM UTF-8 encoder replacement for an unpaired surrogate.
                else -> 3
            }
            if (bytes > limit) return false
        }
        return true
    }

    internal fun redirectURL(current: String, location: String): String {
        // Fragments identify document positions and are never sent in HTTP requests.
        val resolved = URI(current).resolve(location).toString().substringBefore('#')
        return httpsURL(resolved)
    }

    private fun open(url: String, transfer: Transfer, deadline: Long): HttpURLConnection {
        var current = httpsURL(url)
        repeat(6) {
            transfer.check()
            if (System.nanoTime() > deadline) throw IOException("Request timed out")
            val connection = URI(current).toURL().openConnection() as HttpURLConnection
            connection.instanceFollowRedirects = false
            connection.connectTimeout = 15000
            connection.readTimeout = 15000
            connection.setRequestProperty("User-Agent", "PlayCover-A/Repositories")
            connection.setRequestProperty("Accept-Encoding", "identity")
            transfer.attach(connection)
            try {
                val code = connection.responseCode
                if (code in listOf(301, 302, 303, 307, 308)) {
                    val location = connection.getHeaderField("Location") ?: throw IOException("Redirect has no destination")
                    current = redirectURL(current, location)
                    connection.disconnect()
                } else {
                    if (code !in 200..299) throw IOException("Server returned HTTP $code")
                    return connection
                }
            } catch (error: Exception) { connection.disconnect(); throw error }
        }
        throw IOException("Too many redirects")
    }

    fun fetch(url: String, transfer: Transfer): Feed {
        val deadline = System.nanoTime() + 60L * 1000000000
        val connection = open(url, transfer, deadline)
        try {
            require((connection.getHeaderField("Content-Length")?.toLongOrNull() ?: -1) <= FEED_LIMIT) { "Source is too large" }
            val output = java.io.ByteArrayOutputStream()
            connection.inputStream.use { input ->
                val bytes = ByteArray(32768)
                while (true) {
                    transfer.check()
                    if (System.nanoTime() > deadline) throw IOException("Source request timed out")
                    val count = input.read(bytes)
                    if (count == -1) break
                    require(output.size() + count <= FEED_LIMIT) { "Source is too large" }
                    output.write(bytes, 0, count)
                }
            }
            return parseFeed(output.toString("UTF-8"))
        } finally { connection.disconnect() }
    }

    fun fetchIcon(url: String, transfer: Transfer): ByteArray {
        val limit = 2 * 1024 * 1024
        val deadline = System.nanoTime() + 30L * 1000000000
        val connection = open(url, transfer, deadline)
        try {
            require((connection.getHeaderField("Content-Length")?.toLongOrNull() ?: -1) <= limit) { "Icon is too large" }
            val output = java.io.ByteArrayOutputStream()
            connection.inputStream.use { input ->
                val buffer = ByteArray(16384)
                while (true) {
                    transfer.check()
                    if (System.nanoTime() > deadline) throw IOException("Icon request timed out")
                    val count = input.read(buffer)
                    if (count < 0) break
                    require(output.size() + count <= limit) { "Icon is too large" }
                    output.write(buffer, 0, count)
                }
            }
            return output.toByteArray()
        } finally { connection.disconnect() }
    }

    fun download(app: App, directory: File, transfer: Transfer, progress: (Long, Long) -> Unit): File {
        require(directory.isDirectory || directory.mkdirs()) { "Cannot open app library" }
        val temporary = File.createTempFile("repository-", ".partial", directory)
        val deadline = System.nanoTime() + 20L * 60 * 1000000000
        try {
            val connection = open(app.downloadURL, transfer, deadline)
            try {
                val total = connection.getHeaderField("Content-Length")?.toLongOrNull() ?: -1
                require(total <= IPA_LIMIT) { "IPA exceeds the 2 GiB download limit" }
                var received = 0L
                connection.inputStream.use { input -> temporary.outputStream().use { output ->
                    val bytes = ByteArray(65536)
                    while (true) {
                        transfer.check()
                        if (System.nanoTime() > deadline) throw IOException("Download timed out")
                        val count = input.read(bytes)
                        if (count == -1) break
                        received += count
                        require(received <= IPA_LIMIT) { "IPA exceeds the 2 GiB download limit" }
                        output.write(bytes, 0, count)
                        progress(received, total)
                    }
                    output.fd.sync()
                } }
                if (total >= 0 && total != received) throw IOException("Incomplete download")
            } finally { connection.disconnect() }
            transfer.check()
            validateIPA(temporary)
            transfer.check()
            val base = (app.identifier + " " + app.version).replace(Regex("[^\\p{L}\\p{N} ._-]"), "_").take(120).trim('.',' ')
                .ifBlank { "Repository app" }
            synchronized(this) {
                var destination = File(directory, "$base.ipa")
                var number = 2
                while (!destination.createNewFile()) destination = File(directory, "$base (${number++}).ipa")
                // Same-directory rename makes the verified archive appear as one file.
                if (!temporary.renameTo(destination)) { destination.delete(); throw IOException("Cannot save IPA") }
                return destination
            }
        } finally { temporary.delete() }
    }

    fun validateIPA(file: File) {
        ZipFile(file).use { zip ->
            val entries = zip.entries().asSequence().toList()
            require(entries.size <= 100000) { "IPA contains too many entries" }
            require(entries.map { it.name }.toSet().size == entries.size) { "IPA contains duplicate entries" }
            require(entries.none { it.name.startsWith('/') || it.name.contains('\\') ||
                it.name.split('/').any { part -> part == ".." } }) { "Invalid IPA paths" }
            val infos = entries.filter { !it.isDirectory && Regex("Payload/[^/]+\\.app/Info\\.plist").matches(it.name) }
            require(infos.size == 1) { "Download is not an IPA with one application" }
            val info = infos.single()
            require(info.size in 1..1024 * 1024) { "Invalid application metadata size" }
            val bytes = zip.getInputStream(info).use { input ->
                val output = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    require(output.size() + count <= 1024 * 1024) { "Application metadata is too large" }
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            }
            val executable = plistExecutable(bytes)
            require(executable.isNotBlank() && executable.length <= 255 && executable !in listOf(".", "..") &&
                executable.none { it == '/' || it == '\\' || it.isISOControl() }) { "Invalid CFBundleExecutable" }
            val binary = zip.getEntry(info.name.removeSuffix("Info.plist") + executable)
            require(binary != null && !binary.isDirectory && binary.size >= 28) { "Application executable is missing" }
            val magic = zip.getInputStream(binary).use { input -> ByteArray(4).also { java.io.DataInputStream(input).readFully(it) } }
            require(magic.toList() in listOf(
                listOf(0xce.toByte(),0xfa.toByte(),0xed.toByte(),0xfe.toByte()),
                listOf(0xcf.toByte(),0xfa.toByte(),0xed.toByte(),0xfe.toByte()),
                listOf(0xca.toByte(),0xfe.toByte(),0xba.toByte(),0xbe.toByte()),
                listOf(0xca.toByte(),0xfe.toByte(),0xba.toByte(),0xbf.toByte()))) { "Executable is not Mach-O" }
        }
    }

    private fun plistExecutable(bytes: ByteArray): String {
        if (bytes.take(8).toByteArray().contentEquals("bplist00".toByteArray())) return binaryPlistExecutable(bytes)
        val factory = DocumentBuilderFactory.newInstance()
        require(!String(bytes, Charsets.UTF_8).contains("<!ENTITY")) { "Plist entity declarations are unsupported" }
        // Android's bundled parser does not expose all JAXP feature switches.
        runCatching { factory.setFeature("http://xml.org/sax/features/external-general-entities", false) }
        runCatching { factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false) }
        val builder = factory.newDocumentBuilder()
        // Standard Apple XML plists have a DOCTYPE; never fetch that external DTD.
        builder.setEntityResolver { _, _ -> org.xml.sax.InputSource(java.io.StringReader("")) }
        val document = builder.parse(bytes.inputStream())
        val dict = document.documentElement.childNodes
        val root = (0 until dict.length).map { dict.item(it) }.firstOrNull { it.nodeName == "dict" }
            ?: throw IOException("Invalid plist dictionary")
        val children = (0 until root.childNodes.length).map { root.childNodes.item(it) }.filter { it.nodeType == org.w3c.dom.Node.ELEMENT_NODE }
        for (index in 0 until children.size - 1) {
            if (children[index].nodeName == "key" && children[index].textContent == "CFBundleExecutable") {
                require(children[index + 1].nodeName == "string")
                return children[index + 1].textContent
            }
        }
        throw IOException("CFBundleExecutable is missing")
    }

    /** Only the top dictionary's string metadata is needed; all offsets are bounded. */
    private fun binaryPlistExecutable(bytes: ByteArray): String {
        require(bytes.size >= 40)
        fun unsigned(offset: Int, width: Int): Long {
            require(width in 1..8 && offset >= 0 && offset <= bytes.size - width)
            var value = 0L
            repeat(width) { value = (value shl 8) or (bytes[offset + it].toLong() and 255) }
            require(value >= 0) { "Invalid plist integer" }
            return value
        }
        val trailer = bytes.size - 32
        val width = bytes[trailer + 6].toInt() and 255
        val referenceWidth = bytes[trailer + 7].toInt() and 255
        val count = unsigned(trailer + 8, 8)
        val top = unsigned(trailer + 16, 8)
        val table = unsigned(trailer + 24, 8)
        require(count in 1..100000 && top < count && width in 1..8 && referenceWidth in 1..8)
        require(table in 8..trailer.toLong() && count * width <= trailer - table)
        fun offset(reference: Long): Int {
            require(reference in 0 until count)
            val result = unsigned((table + reference * width).toInt(), width)
            require(result in 8 until table)
            return result.toInt()
        }
        fun length(position: Int): Pair<Int, Int> {
            val small = bytes[position].toInt() and 15
            if (small < 15) return Pair(small, position + 1)
            val marker = bytes[position + 1].toInt() and 255
            require(marker ushr 4 == 1 && marker and 15 <= 3)
            val size = 1 shl (marker and 15)
            val value = unsigned(position + 2, size)
            require(value <= 100000)
            return Pair(value.toInt(), position + 2 + size)
        }
        fun string(reference: Long): String {
            val position = offset(reference)
            val kind = (bytes[position].toInt() and 255) ushr 4
            require(kind == 5 || kind == 6)
            val (length, start) = length(position)
            val size = length * if (kind == 6) 2 else 1
            require(start + size <= table)
            return String(bytes, start, size, if (kind == 6) Charsets.UTF_16BE else Charsets.US_ASCII)
        }
        val root = offset(top)
        require((bytes[root].toInt() and 255) ushr 4 == 13)
        val (pairs, start) = length(root)
        require(start.toLong() + 2L * pairs * referenceWidth <= table)
        for (index in 0 until pairs) {
            if (string(unsigned(start + index * referenceWidth, referenceWidth)) == "CFBundleExecutable") {
                return string(unsigned(start + (pairs + index) * referenceWidth, referenceWidth))
            }
        }
        throw IOException("CFBundleExecutable is missing")
    }
}
