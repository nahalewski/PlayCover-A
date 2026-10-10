/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayInputStream
import java.io.File
import java.nio.ByteBuffer
import java.util.zip.Inflater
import java.util.zip.ZipFile

/**
 * What the launcher shows for an app: its real name and its icon, read from
 * inside the .ipa (a zip file). Everything here is best-effort: any failure
 * just means the launcher falls back to the file name and a letter tile.
 */
class IpaInfo(val displayName: String?, val icon: Bitmap?,
              val minimumIosVersion: String? = null, val is64Bit: Boolean? = null,
              val scan: SymbolIndex.Report? = null, val bundleId: String? = null) {
    companion object {
        fun read(ipa: File): IpaInfo? {
            if (!ipa.isFile) return null
            return try {
                ZipFile(ipa).use { zip ->
                    val entries = zip.entries().toList()
                    val infoEntry = entries.firstOrNull {
                        Regex("^Payload/[^/]+\\.app/Info\\.plist$").matches(it.name)
                    } ?: return null
                    val appDir = infoEntry.name.removeSuffix("Info.plist")
                    val plist = Plist.parse(zip.getInputStream(infoEntry).use { it.readBytesLimited(1024 * 1024) }) as? Map<*, *>
                        ?: return IpaInfo(null, null)

                    val name = (plist["CFBundleDisplayName"] as? String)?.takeIf { it.isNotBlank() }
                        ?: (plist["CFBundleName"] as? String)?.takeIf { it.isNotBlank() }
                    val infoMinimum = canonicalIosVersion(plist["MinimumOSVersion"] as? String)
                    val executable = (plist["CFBundleExecutable"] as? String)?.takeIf {
                        it.isNotEmpty() && it.length <= 255 && !it.contains('/') && it != "." && it != ".."
                    }?.let { zip.getEntry(appDir + it) }
                    val sequential = executable?.let { SequentialZipReader(zip, it) }
                    val macho = try {
                        sequential?.let { reader -> readMachOMetadata { offset, count -> reader.read(offset, count) } }
                    } catch (_: Exception) { null }
                    val scan = try {
                        if (macho != null && !macho.is64Bit && SymbolIndex.ensureLoaded()) {
                            val key = "${ipa.name}|${ipa.length()}|${ipa.lastModified()}|${SymbolIndex.version}"
                            ScanCache.get(key) ?: sequential?.let { reader ->
                                readUndefinedSymbols { offset, count -> reader.read(offset, count) }?.let { SymbolIndex.check(it) }
                            }?.also { ScanCache.put(key, it) }
                        } else null
                    } catch (_: Throwable) { null }

                    // Collect the icon file names the plist mentions, in all the
                    // ways iOS versions have spelled it.
                    val wanted = ArrayList<String>()
                    (plist["CFBundleIconFiles"] as? List<*>)?.forEach { (it as? String)?.let(wanted::add) }
                    (plist["CFBundleIconFile"] as? String)?.let(wanted::add)
                    (((plist["CFBundleIcons"] as? Map<*, *>)?.get("CFBundlePrimaryIcon")
                        as? Map<*, *>)?.get("CFBundleIconFiles") as? List<*>)
                        ?.forEach { (it as? String)?.let(wanted::add) }

                    // Candidate files: the named ones (with or without ".png"),
                    // or else anything called Icon*.png in the app folder.
                    val inApp = entries.filter {
                        it.name.startsWith(appDir) && !it.name.removePrefix(appDir).contains('/')
                    }
                    fun matchesName(entryName: String, wantedName: String): Boolean {
                        val base = entryName.removePrefix(appDir)
                        val stem = wantedName.removeSuffix(".png")
                        return base.equals(wantedName, true) ||
                            base.startsWith(stem, true) && base.endsWith(".png", true)
                    }
                    // Exact names first: a loose prefix match lets files like
                    // "icon_background.png" beat the real "Icon-114.png" by size.
                    fun exactName(entryName: String, wantedName: String): Boolean {
                        val base = entryName.removePrefix(appDir).lowercase()
                        val stem = wantedName.lowercase().removeSuffix(".png")
                        return base == "$stem.png" || base == "${stem}@2x.png" || base == "$stem~ipad.png" || base == "${stem}@2x~ipad.png"
                    }
                    var candidates = inApp.filter { e -> wanted.any { exactName(e.name, it) } }
                    if (candidates.isEmpty()) candidates = inApp.filter { e -> wanted.any { matchesName(e.name, it) } }
                    if (candidates.isEmpty()) {
                        candidates = inApp.filter {
                            val base = it.name.removePrefix(appDir).lowercase()
                            base.startsWith("icon") && base.endsWith(".png")
                        }
                    }
                    // Use the biggest file: that's the highest resolution one.
                    val icon = candidates.sortedByDescending { it.size }.firstNotNullOfOrNull { entry ->
                        try {
                            decodePng(zip.getInputStream(entry).use { it.readBytesLimited(4 * 1024 * 1024) })
                        } catch (error: Exception) {
                            android.util.Log.w("IpaInfo", "Cannot read icon ${entry.name}", error)
                            null
                        }
                    }
                    if (icon == null) android.util.Log.w("IpaInfo", "No decodable icon in ${ipa.name} (${candidates.size} candidates)")
                    IpaInfo(name, icon, highestIosVersion(infoMinimum, macho?.minimum), macho?.is64Bit, scan,
                        (plist["CFBundleIdentifier"] as? String))
                }
            } catch (e: Throwable) {
                android.util.Log.w("IpaInfo", "Cannot read ${ipa.name}", e)
                null
            }
        }

        internal fun canonicalIosVersion(value: String?): String? {
            if (value == null || value.length > 16 || !Regex("[0-9]{1,3}(\\.[0-9]{1,3}){0,2}").matches(value)) return null
            val parts = value.split('.').map { it.toInt() }
            if (parts[0] !in 1..255 || parts.drop(1).any { it > 255 }) return null
            return (if (parts.size == 1) parts + 0 else parts).joinToString(".")
        }

        internal fun minimumIosVersionFromPlist(bytes: ByteArray): String? =
            canonicalIosVersion((Plist.parse(bytes) as? Map<*, *>)?.get("MinimumOSVersion") as? String)

        internal fun highestIosVersion(a: String?, b: String?): String? {
            if (a == null) return b
            if (b == null) return a
            val av = a.split('.').map { it.toInt() }; val bv = b.split('.').map { it.toInt() }
            for (i in 0..2) {
                val delta = (av.getOrNull(i) ?: 0).compareTo(bv.getOrNull(i) ?: 0)
                if (delta != 0) return if (delta > 0) a else b
            }
            return a
        }

        internal data class MachOMetadata(val minimum: String?, val is64Bit: Boolean)

        /** Reads only the selected ARM header and bounded load commands. */
        internal fun readMachOMetadata(read: (Long, Int) -> ByteArray): MachOMetadata? {
            val start = read(0, 8)
            val magic = ByteBuffer.wrap(start).int
            var offset = 0L
            var sliceSize: Long? = null
            var selectedCpu: Int? = null
            var hasArm64Slice = false
            if (magic == 0xcafebabe.toInt() || magic == 0xcafebabf.toInt()) {
                val count = ByteBuffer.wrap(start).getInt(4)
                require(count in 1..32)
                val wide = magic == 0xcafebabf.toInt()
                val stride = if (wide) 32 else 20
                val table = ByteBuffer.wrap(read(8, count * stride))
                val slices = (0 until count).map { i ->
                    val at = i * stride
                    val cpu = table.getInt(at)
                    val base = if (wide) table.getLong(at + 8) else table.getInt(at + 8).toLong() and 0xffffffffL
                    val size = if (wide) table.getLong(at + 16) else table.getInt(at + 12).toLong() and 0xffffffffL
                    Triple(cpu, base, size)
                }
                hasArm64Slice = slices.any { it.first == 0x100000c }
                val selected = slices.firstOrNull { it.first == 12 } ?: slices.firstOrNull { it.first == 0x100000c } ?: return null
                require(selected.second >= 8L + count * stride && selected.third >= 28 && selected.second <= 128L * 1024 * 1024)
                offset = selected.second; sliceSize = selected.third; selectedCpu = selected.first
            }
            val header = ByteBuffer.wrap(read(offset, 28)).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            val thinMagic = header.getInt(0)
            val wide = thinMagic == 0xfeedfacf.toInt()
            if (!wide && thinMagic != 0xfeedface.toInt()) return null
            val cpu = header.getInt(4)
            if (cpu != if (wide) 0x100000c else 12) return null
            if (selectedCpu != null && cpu != selectedCpu) return null
            val count = header.getInt(16); val size = header.getInt(20)
            require(count in 0..4096 && size in 0..(1024 * 1024))
            val headerSize = if (wide) 32 else 28
            require(sliceSize == null || headerSize.toLong() + size <= sliceSize)
            val commands = ByteBuffer.wrap(read(offset + headerSize, size)).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            var cursor = 0; var minimum: String? = null
            repeat(count) {
                require(cursor <= size - 8)
                val command = commands.getInt(cursor); val length = commands.getInt(cursor + 4)
                require(length >= 8 && length % (if (wide) 8 else 4) == 0 && length <= size - cursor)
                val version = when {
                    command == 0x25 -> { require(length >= 16); commands.getInt(cursor + 8) }
                    command == 0x32 -> { require(length >= 24); if (commands.getInt(cursor + 8) == 2) commands.getInt(cursor + 12) else null }
                    else -> null
                }
                version?.let {
                    val text = "${it ushr 16}.${(it ushr 8) and 255}.${it and 255}"
                    minimum = highestIosVersion(minimum, canonicalIosVersion(text))
                }
                cursor += length
            }
            require(cursor == size)
            return MachOMetadata(minimum, wide || hasArm64Slice)
        }

        /** Names of the external symbols an armv7 app imports (from its symbol table). */
        internal fun readUndefinedSymbols(read: (Long, Int) -> ByteArray): Set<String>? {
            val start = read(0, 8)
            val magic = ByteBuffer.wrap(start).int
            var base = 0L
            if (magic == 0xcafebabe.toInt()) {
                val count = ByteBuffer.wrap(start).getInt(4)
                require(count in 1..32)
                val table = ByteBuffer.wrap(read(8, count * 20))
                val slice = (0 until count).map { i -> table.getInt(i * 20) to (table.getInt(i * 20 + 8).toLong() and 0xffffffffL) }
                    .firstOrNull { it.first == 12 } ?: return null
                base = slice.second
            }
            val header = ByteBuffer.wrap(read(base, 28)).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            if (header.getInt(0) != 0xfeedface.toInt()) return null
            val count = header.getInt(16); val size = header.getInt(20)
            require(count in 0..4096 && size in 0..(1024 * 1024))
            val commands = ByteBuffer.wrap(read(base + 28, size)).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            var cursor = 0
            var symOff = 0L; var nsyms = 0; var strOff = 0L; var strSize = 0
            repeat(count) {
                require(cursor <= size - 8)
                val command = commands.getInt(cursor); val length = commands.getInt(cursor + 4)
                require(length >= 8 && length <= size - cursor)
                if (command == 2) { // LC_SYMTAB
                    symOff = commands.getInt(cursor + 8).toLong() and 0xffffffffL
                    nsyms = commands.getInt(cursor + 12)
                    strOff = commands.getInt(cursor + 16).toLong() and 0xffffffffL
                    strSize = commands.getInt(cursor + 20)
                }
                cursor += length
            }
            if (nsyms <= 0 || nsyms > 400_000 || strSize <= 0 || strSize > 24 * 1024 * 1024) return null
            val symbols = ByteBuffer.wrap(read(base + symOff, nsyms * 12)).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            val strings = read(base + strOff, strSize)
            val out = HashSet<String>()
            for (i in 0 until nsyms) {
                val at = i * 12
                val strx = symbols.getInt(at)
                val type = symbols.get(at + 4).toInt() and 0xff
                // External (N_EXT) and undefined (N_UNDF), and not a debug entry.
                if ((type and 0xe0) != 0 || (type and 0x0e) != 0 || (type and 1) == 0) continue
                if (strx <= 0 || strx >= strings.size) continue
                var end = strx
                while (end < strings.size && strings[end] != 0.toByte()) end++
                out.add(String(strings, strx, end - strx, Charsets.US_ASCII))
            }
            return out
        }

        /** Decodes a PNG, including Apple's optimised "CgBI" variant that Android can't. */
        private fun decodePng(data: ByteArray): Bitmap? {
            return try {
                // Android's PNG decoder may partially accept CgBI without undoing
                // its raw-deflate/BGRA transformation. Select by format first.
                if (isCgBI(data)) {
                    val pixels = decodeCgBIPixels(data) ?: return null
                    Bitmap.createBitmap(pixels.argb, pixels.width, pixels.height, Bitmap.Config.ARGB_8888)
                } else {
                    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                    BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
                    if (bounds.outWidth <= 0 || bounds.outHeight <= 0 || bounds.outWidth.toLong() * bounds.outHeight > 4_000_000) return null
                    BitmapFactory.decodeByteArray(data, 0, data.size)
                }
            } catch (e: Throwable) {
                android.util.Log.w("IpaInfo", "Cannot decode icon PNG", e)
                null
            }
        }

        private fun java.io.InputStream.readBytesLimited(limit: Int): ByteArray {
            val output = java.io.ByteArrayOutputStream()
            val block = ByteArray(8192)
            while (true) {
                val count = read(block)
                if (count < 0) break
                require(output.size().toLong() + count <= limit) { "Icon metadata exceeds limit" }
                output.write(block, 0, count)
            }
            return output.toByteArray()
        }

        internal fun isCgBI(data: ByteArray): Boolean = data.size >= 16 &&
            data.copyOfRange(0, 8).contentEquals(byteArrayOf(137.toByte(), 80, 78, 71, 13, 10, 26, 10)) &&
            String(data, 12, 4, Charsets.US_ASCII) == "CgBI"

        internal data class IconPixels(val width: Int, val height: Int, val argb: IntArray)

        /**
         * Apple's Xcode rewrites app PNGs ("CgBI"): the pixels are BGRA with the
         * alpha premultiplied, and the zlib data has no header or checksum.
         * This undoes all that to get an ordinary bitmap.
         */
        internal fun decodeCgBIPixels(data: ByteArray): IconPixels? {
            if (!isCgBI(data)) return null
            val buf = ByteBuffer.wrap(data)
            buf.position(8) // PNG signature
            var width = 0
            var height = 0
            var colorType = 0
            val idat = java.io.ByteArrayOutputStream()
            while (buf.remaining() >= 12) {
                val length = buf.int
                if (length < 0 || length > buf.remaining() - 8) return null
                val type = ByteArray(4).also { buf.get(it) }.toString(Charsets.US_ASCII)
                val chunk = ByteArray(length).also { buf.get(it) }
                buf.int // CRC
                when (type) {
                    "IHDR" -> {
                        if (chunk.size != 13) return null
                        val ihdr = ByteBuffer.wrap(chunk)
                        width = ihdr.int
                        height = ihdr.int
                        val bitDepth = ihdr.get().toInt()
                        colorType = ihdr.get().toInt()
                        ihdr.get() // compression
                        ihdr.get() // filter method
                        val interlace = ihdr.get().toInt()
                        if (bitDepth != 8 || colorType != 6 || interlace != 0) return null
                    }
                    "IDAT" -> idat.write(chunk)
                    "IEND" -> break
                }
            }
            if (width <= 0 || height <= 0 || width.toLong() * height > 4_000_000 || colorType != 6) return null

            val stride = width * 4
            val raw = ByteArray((stride + 1) * height)
            val inflater = Inflater(true) // raw deflate: no zlib header
            var filled = 0
            try {
                inflater.setInput(idat.toByteArray())
                while (filled < raw.size) {
                    val n = inflater.inflate(raw, filled, raw.size - filled)
                    if (n == 0) return null // Never spin on malformed/truncated deflate.
                    filled += n
                }
                val extra = ByteArray(1)
                if (inflater.inflate(extra) != 0 || !inflater.finished()) return null
            } finally { inflater.end() }

            // Undo the PNG row filters (4 bytes per pixel).
            val pixels = ByteArray(stride * height)
            for (y in 0 until height) {
                val filter = raw[y * (stride + 1)].toInt()
                val src = y * (stride + 1) + 1
                val dst = y * stride
                for (x in 0 until stride) {
                    val cur = raw[src + x].toInt() and 0xff
                    val left = if (x >= 4) pixels[dst + x - 4].toInt() and 0xff else 0
                    val up = if (y > 0) pixels[dst - stride + x].toInt() and 0xff else 0
                    val upLeft = if (y > 0 && x >= 4) pixels[dst - stride + x - 4].toInt() and 0xff else 0
                    val value = when (filter) {
                        0 -> cur
                        1 -> cur + left
                        2 -> cur + up
                        3 -> cur + (left + up) / 2
                        4 -> {
                            val p = left + up - upLeft
                            val pa = Math.abs(p - left)
                            val pb = Math.abs(p - up)
                            val pc = Math.abs(p - upLeft)
                            cur + if (pa <= pb && pa <= pc) left else if (pb <= pc) up else upLeft
                        }
                        else -> return null
                    }
                    pixels[dst + x] = value.toByte()
                }
            }

            // BGRA premultiplied -> ARGB straight.
            val argb = IntArray(width * height)
            for (i in argb.indices) {
                val b = pixels[i * 4].toInt() and 0xff
                val g = pixels[i * 4 + 1].toInt() and 0xff
                val r = pixels[i * 4 + 2].toInt() and 0xff
                val a = pixels[i * 4 + 3].toInt() and 0xff
                fun unpremultiply(c: Int) = if (a == 0) 0 else Math.min(255, c * 255 / a)
                argb[i] = (a shl 24) or (unpremultiply(r) shl 16) or (unpremultiply(g) shl 8) or unpremultiply(b)
            }
            return IconPixels(width, height, argb)
        }
    }
}

/** A small reader for Apple property lists, binary ("bplist00") or XML. */
private object Plist {
    fun parse(data: ByteArray): Any? =
        if (data.size > 8 && String(data, 0, 8, Charsets.US_ASCII) == "bplist00") {
            Binary(data).parse()
        } else {
            parseXml(data)
        }

    private class Binary(private val d: ByteArray) {
        private val offsetSize: Int
        private val refSize: Int
        private val numObjects: Int
        private val topObject: Int
        private val offsetTable: Int

        init {
            val t = d.size - 32
            offsetSize = d[t + 6].toInt() and 0xff
            refSize = d[t + 7].toInt() and 0xff
            numObjects = readInt(t + 8, 8).toInt()
            topObject = readInt(t + 16, 8).toInt()
            offsetTable = readInt(t + 24, 8).toInt()
        }

        private fun readInt(pos: Int, size: Int): Long {
            var v = 0L
            for (i in 0 until size) v = (v shl 8) or (d[pos + i].toLong() and 0xff)
            return v
        }

        fun parse(): Any? = obj(topObject)

        private fun obj(index: Int): Any? {
            val offset = readInt(offsetTable + index * offsetSize, offsetSize).toInt()
            val marker = d[offset].toInt() and 0xff
            val type = marker shr 4
            val low = marker and 0xf
            // Length: in the low nibble, or (if 0xf) in a following int object.
            fun lengthAndStart(): Pair<Int, Int> {
                if (low != 0xf) return Pair(low, offset + 1)
                val sizeMarker = d[offset + 1].toInt() and 0xff
                val bytes = 1 shl (sizeMarker and 0xf)
                return Pair(readInt(offset + 2, bytes).toInt(), offset + 2 + bytes)
            }
            return when (type) {
                0x0 -> when (low) { 0x8 -> false; 0x9 -> true; else -> null }
                0x1 -> readInt(offset + 1, 1 shl low)
                0x2 -> {
                    val bits = readInt(offset + 1, 1 shl low)
                    if (low == 2) java.lang.Float.intBitsToFloat(bits.toInt()).toDouble()
                    else java.lang.Double.longBitsToDouble(bits)
                }
                0x4 -> { val (n, s) = lengthAndStart(); d.copyOfRange(s, s + n) }
                0x5 -> { val (n, s) = lengthAndStart(); String(d, s, n, Charsets.US_ASCII) }
                0x6 -> { val (n, s) = lengthAndStart(); String(d, s, n * 2, Charsets.UTF_16BE) }
                0xA -> {
                    val (n, s) = lengthAndStart()
                    List(n) { obj(readInt(s + it * refSize, refSize).toInt()) }
                }
                0xD -> {
                    val (n, s) = lengthAndStart()
                    val map = LinkedHashMap<String, Any?>()
                    for (i in 0 until n) {
                        val key = obj(readInt(s + i * refSize, refSize).toInt()) as? String ?: continue
                        map[key] = obj(readInt(s + (n + i) * refSize, refSize).toInt())
                    }
                    map
                }
                else -> null
            }
        }
    }

    private fun parseXml(data: ByteArray): Any? {
        val parser = android.util.Xml.newPullParser()
        parser.setInput(ByteArrayInputStream(data), null)
        fun readText(): String {
            var text = ""
            var event = parser.next()
            while (event != org.xmlpull.v1.XmlPullParser.END_TAG) {
                if (event == org.xmlpull.v1.XmlPullParser.TEXT) text += parser.text
                event = parser.next()
            }
            return text
        }
        fun readValue(): Any? {
            // Positioned at the start tag of a value.
            return when (parser.name) {
                "dict" -> {
                    val map = LinkedHashMap<String, Any?>()
                    var key: String? = null
                    while (parser.next() != org.xmlpull.v1.XmlPullParser.END_TAG) {
                        if (parser.eventType != org.xmlpull.v1.XmlPullParser.START_TAG) continue
                        if (parser.name == "key") key = readText()
                        else { val v = readValue(); key?.let { map[it] = v } }
                    }
                    map
                }
                "array" -> {
                    val list = ArrayList<Any?>()
                    while (parser.next() != org.xmlpull.v1.XmlPullParser.END_TAG) {
                        if (parser.eventType == org.xmlpull.v1.XmlPullParser.START_TAG) list.add(readValue())
                    }
                    list
                }
                "string" -> readText()
                "integer" -> readText().trim().toLongOrNull()
                "true" -> { readText(); true }
                "false" -> { readText(); false }
                else -> { readText(); null }
            }
        }
        var event = parser.eventType
        while (event != org.xmlpull.v1.XmlPullParser.END_DOCUMENT) {
            if (event == org.xmlpull.v1.XmlPullParser.START_TAG && parser.name == "dict") return readValue()
            event = parser.next()
        }
        return null
    }
}

/**
 * Reads a zip entry at increasing offsets from a single inflate pass; going
 * backwards (rare, and then only to the start of the file) reopens the entry.
 */
internal class SequentialZipReader(private val zip: ZipFile, private val entry: java.util.zip.ZipEntry) {
    private var stream: java.io.InputStream? = null
    private var position = 0L

    fun read(offset: Long, count: Int): ByteArray {
        require(offset >= 0 && count >= 0 && offset <= 512L * 1024 * 1024)
        if (stream == null || offset < position) {
            stream?.close()
            stream = zip.getInputStream(entry)
            position = 0
        }
        val input = stream!!
        while (position < offset) {
            val skipped = input.skip(offset - position)
            if (skipped > 0) position += skipped
            else { require(input.read() >= 0); position++ }
        }
        val bytes = ByteArray(count)
        var done = 0
        while (done < count) {
            val n = input.read(bytes, done, count - done)
            require(n > 0)
            done += n
        }
        position += count
        return bytes
    }
}
