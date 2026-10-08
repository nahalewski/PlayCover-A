package org.touchhle.android

import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.AtomicFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.io.RandomAccessFile
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Durable downloads. Android force-stop requires reopening the application to resume. */
object IpaDownloads {
    data class Job(val id: String, val name: String, val iconURL: String?, val status: String,
                   val received: Long, val total: Long, val error: String?, val outputFilename: String?)
    private data class Record(val id: String, val name: String, val icon: String?, val url: String,
                              val target: String, var status: String = "QUEUED", var received: Long = 0,
                              var total: Long = -1, var validator: String? = null, var error: String? = null,
                              var generation: Long = 0)
    private class Work(val generation: Long) {
        val cancelled = AtomicBoolean(false)
        @Volatile var connection: HttpURLConnection? = null
        fun check() { if (cancelled.get()) throw IOException("Paused") }
    }
    private val records = linkedMapOf<String, Record>()
    private val running = mutableMapOf<String, Work>()
    private val listeners = CopyOnWriteArraySet<() -> Unit>()
    private val workers = Executors.newFixedThreadPool(2)
    private val main = Handler(Looper.getMainLooper())
    private var context: Context? = null
    private var loaded = false
    private const val LIMIT = 2L * 1024 * 1024 * 1024
    private fun root(c: Context) = File(c.getExternalFilesDir(null) ?: throw IOException("External storage unavailable"), "touchHLE_apps").also { it.mkdirs() }
    private fun part(c: Context, id: String) = File(File(root(c), ".downloads").also { it.mkdirs() }, "$id.part")
    private fun store(c: Context) = AtomicFile(File(c.filesDir, "ipa-downloads.json"))
    private fun notifyChanged() { main.post { listeners.forEach { listener ->
        try { listener() } catch (e: Exception) { android.util.Log.w("IpaDownloads", "Download observer failed", e) }
    } } }
    @Synchronized private fun load(c: Context) {
        context = c.applicationContext
        if (loaded) return
        val file = store(c)
        if (file.baseFile.exists()) {
            try {
                val bytes = file.openRead().use { input ->
                    val out = java.io.ByteArrayOutputStream()
                    val buffer = ByteArray(8192)
                    while (true) { val n = input.read(buffer); if (n < 0) break; require(out.size() + n <= 4 * 1024 * 1024); out.write(buffer, 0, n) }
                    out.toByteArray()
                }
                val array = JSONArray(String(bytes, Charsets.UTF_8))
                require(array.length() <= 200)
                for (i in 0 until array.length()) {
                    val v = array.getJSONObject(i)
                    val id = v.getString("id"); UUID.fromString(id)
                    val target = v.getString("target")
                    require(target == File(target).name && target.endsWith(".ipa") && target.length <= 240)
                    val r = Record(id, v.getString("name").take(512), v.optString("icon").takeIf { it.isNotEmpty() },
                        RepoSources.httpsURL(v.getString("url")), target,
                        v.optString("status", "PAUSED"), v.optLong("received"), v.optLong("total", -1),
                        v.optString("validator").takeIf { it.isNotEmpty() }, v.optString("error").takeIf { it.isNotEmpty() })
                    if (r.status == "RUNNING") r.status = "QUEUED"
                    if (r.status !in setOf("QUEUED", "COMPLETE", "PAUSED", "FAILED")) r.status = "PAUSED"
                    // Removing an installed IPA is intentional; completed history must not
                    // resurrect it as a failed download after the next process restart.
                    if (!DownloadProtocol.restoreRecord(r.status, File(root(c), target).isFile)) continue
                    records[id] = r
                }
            } catch (e: Exception) {
                android.util.Log.w("IpaDownloads", "Cannot restore download queue", e)
                records.clear()
            }
        }
        loaded = true
    }
    @Synchronized private fun persist() {
        val c = context ?: return
        val array = JSONArray()
        records.values.forEach { r -> array.put(JSONObject().put("id", r.id).put("name", r.name)
            .put("icon", r.icon ?: "").put("url", r.url).put("target", r.target).put("status", r.status)
            .put("received", r.received).put("total", r.total).put("validator", r.validator ?: "").put("error", r.error ?: "")) }
        val file = store(c)
        val output = file.startWrite()
        try { output.write(array.toString().toByteArray(Charsets.UTF_8)); file.finishWrite(output) }
        catch (e: Exception) { file.failWrite(output); throw e }
    }
    private fun start(c: Context) {
        val intent = Intent(c, IpaDownloadService::class.java)
        if (Build.VERSION.SDK_INT >= 26) c.startForegroundService(intent) else c.startService(intent)
    }
    @Synchronized fun enqueue(c: Context, app: RepoSources.App): String {
        load(c)
        records.values.firstOrNull { it.url == app.downloadURL && it.status in setOf("QUEUED", "RUNNING") }?.let { return it.id }
        // Completed records are history only. Retire the oldest metadata when a
        // new transfer needs room, while leaving every installed IPA untouched.
        val retire = DownloadProtocol.historyToRetire(records.values.map { it.id to it.status }, 200)
        retire.forEach { records.remove(it) }
        check(records.size < 200) { "Pause or finish existing downloads before adding more" }
        val id = UUID.randomUUID().toString()
        val name = app.name.replace(Regex("[^A-Za-z0-9 ._-]"), "_").take(90).ifBlank { "App" }
        val target = "$name-${id.take(8)}.ipa"
        records[id] = Record(id, app.name.take(512), app.iconURL, RepoSources.httpsURL(app.downloadURL), target)
        persist(); notifyChanged(); start(c.applicationContext)
        return id
    }
    @Synchronized fun snapshot(c: Context): List<Job> {
        load(c)
        return records.values.map { Job(it.id, it.name, it.icon, it.status, it.received, it.total, it.error,
            if (it.status == "COMPLETE") it.target else null) }
    }
    fun addListener(listener: () -> Unit) { listeners.add(listener) }
    fun removeListener(listener: () -> Unit) { listeners.remove(listener) }
    @Synchronized fun cancel(c: Context, id: String) {
        load(c); val r = records[id] ?: return
        if (r.status == "COMPLETE") return
        r.generation++; r.status = "PAUSED"; r.error = null
        running[id]?.let { it.cancelled.set(true); it.connection?.disconnect() }
        persist(); notifyChanged()
    }
    @Synchronized fun retry(c: Context, id: String) {
        load(c); val r = records[id] ?: return
        if (r.status !in setOf("PAUSED", "FAILED")) return
        r.generation++; r.status = "QUEUED"; r.error = null
        persist(); notifyChanged(); start(c.applicationContext)
    }
    @Synchronized fun resume(c: Context) {
        load(c)
        if (records.values.any { it.status == "QUEUED" }) start(c.applicationContext)
    }
    /** Called only once the foreground service has posted its notification. */
    @Synchronized internal fun schedule(c: Context) {
        load(c)
        for (r in records.values) {
            if (running.size >= 2) break
            if (r.status != "QUEUED" || running.containsKey(r.id)) continue
            val work = Work(r.generation); running[r.id] = work; r.status = "RUNNING"
            android.util.Log.i("IpaDownloads", "Started ${r.id}; active slots=${running.size}")
            persist(); notifyChanged()
            workers.execute { run(c.applicationContext, r.id, work) }
        }
    }
    @Synchronized internal fun active(): Boolean = running.isNotEmpty() || records.values.any { it.status == "QUEUED" }
    @Synchronized private fun update(id: String, work: Work, action: (Record) -> Unit) {
        val r = records[id] ?: return
        work.check()
        if (r.generation != work.generation) throw IOException("Superseded download")
        action(r); persist(); notifyChanged()
    }
    private fun run(c: Context, id: String, work: Work) {
        try {
            var attempt = 0
            while (true) {
                try { transfer(c, id, work); break }
                catch (e: IOException) {
                    work.check()
                    if (e !is NetworkFailure) throw e
                    update(id, work) { it.error = "Waiting for connection; retrying automatically" }
                    val until = System.currentTimeMillis() + DownloadProtocol.retryDelay(attempt)
                    attempt = (attempt + 1).coerceAtMost(6)
                    while (System.currentTimeMillis() < until) { work.check(); Thread.sleep(100) }
                }
            }
        } catch (e: Exception) {
            synchronized(this) {
                records[id]?.takeIf { it.generation == work.generation && !work.cancelled.get() }?.let {
                    it.status = "FAILED"; it.error = e.message?.take(512) ?: "Download failed"
                    try { persist() } catch (ignored: Exception) { android.util.Log.w("IpaDownloads", "Cannot save download failure", ignored) }
                }
            }
        } finally {
            work.connection?.disconnect()
            synchronized(this) { if (running[id] === work) running.remove(id) }
            notifyChanged(); schedule(c)
        }
    }
    private class PermanentFailure(message: String) : IOException(message)
    private class NetworkFailure(message: String?, cause: Throwable? = null) : IOException(message, cause)
    private fun <T> network(action: () -> T): T = try { action() }
        catch (e: javax.net.ssl.SSLHandshakeException) { throw PermanentFailure("HTTPS certificate handshake failed: ${e.message}") }
        catch (e: javax.net.ssl.SSLPeerUnverifiedException) { throw PermanentFailure("HTTPS certificate could not be verified") }
        catch (e: IOException) { throw NetworkFailure(e.message, e) }
    private fun transfer(c: Context, id: String, work: Work) {
        val r = synchronized(this) { records.getValue(id).copy() }
        val file = part(c, id)
        // Recover a death after the atomic rename but before the job checkpoint.
        val completed = File(root(c), r.target)
        if (completed.isFile && !file.exists()) {
            RepoSources.validateIPA(completed)
            update(id, work) { it.status = "COMPLETE"; it.received = completed.length(); it.total = completed.length(); it.error = null }
            return
        }
        var offset = file.length()
        if (offset > LIMIT) throw PermanentFailure("Partial download exceeds 2 GiB")
        if (offset > 0 && r.validator == null) { RandomAccessFile(file, "rw").use { it.setLength(0) }; offset = 0 }
        var current = r.url
        var conn: HttpURLConnection? = null
        for (redirect in 0..6) {
            work.check()
            val candidate = network { URL(current).openConnection() as HttpURLConnection }
            conn = candidate; work.connection = candidate
            candidate.instanceFollowRedirects = false; candidate.connectTimeout = 15000; candidate.readTimeout = 15000
            candidate.setRequestProperty("Accept-Encoding", "identity")
            if (offset > 0) { candidate.setRequestProperty("Range", "bytes=$offset-"); candidate.setRequestProperty("If-Range", r.validator) }
            val status = network { candidate.responseCode }
            if (status !in listOf(301, 302, 303, 307, 308)) break
            val location = candidate.getHeaderField("Location") ?: throw PermanentFailure("Redirect has no destination")
            candidate.disconnect()
            if (redirect == 6) throw PermanentFailure("Too many redirects")
            current = RepoSources.redirectURL(current, location)
        }
        val connection = conn ?: throw IOException("No response")
        try {
            val status = network { connection.responseCode }
            android.util.Log.i("IpaDownloads", "Response ${r.id}: offset=$offset status=$status")
            if (status !in listOf(200, 206)) {
                if (status == 416) {
                    val total = DownloadProtocol.unsatisfiedLength(connection.getHeaderField("Content-Range"))
                    val entity = DownloadProtocol.validator(connection.getHeaderField("ETag"), connection.getHeaderField("Last-Modified"))
                    if (DownloadProtocol.completeRange(offset, total, r.validator, entity)) { finish(c, id, work, file, r.target); return }
                    RandomAccessFile(file, "rw").use { it.setLength(0) }
                    update(id, work) { it.validator = null; it.received = 0 }
                    throw NetworkFailure("Resume range expired; restarting")
                }
                if (status == 408 || status == 429 || status in 500..599) throw NetworkFailure("HTTP $status")
                throw PermanentFailure("HTTP $status")
            }
            val validator = DownloadProtocol.validator(connection.getHeaderField("ETag"), connection.getHeaderField("Last-Modified"))
            if (connection.getHeaderField("Content-Encoding")?.let { !it.equals("identity", true) } == true)
                throw PermanentFailure("Encoded IPA response cannot be safely resumed")
            val length = connection.getHeaderField("Content-Length")?.toLongOrNull() ?: -1
            val decision = try { DownloadProtocol.response(status, offset, connection.getHeaderField("Content-Range"), length, r.validator, validator) }
            catch (e: IOException) {
                RandomAccessFile(file, "rw").use { it.setLength(0); it.fd.sync() }
                update(id, work) { it.validator = null; it.received = 0; it.total = -1 }
                throw NetworkFailure(e.message, e)
            }
            if (decision.total > LIMIT) throw PermanentFailure("IPA exceeds 2 GiB")
            offset = decision.offset
            RandomAccessFile(file, "rw").use { output ->
                if (offset == 0L) output.setLength(0)
                output.seek(offset)
                update(id, work) { it.validator = validator; it.total = decision.total; it.received = offset; it.error = null }
                var received = offset; var checkpoint = System.currentTimeMillis()
                val deadline = checkpoint + 20 * 60 * 1000
                network { connection.inputStream }.use { input ->
                    val bytes = ByteArray(65536)
                    while (true) {
                        work.check()
                        if (System.currentTimeMillis() > deadline) throw NetworkFailure("Download connection timed out; resuming")
                        val n = network { input.read(bytes) }; if (n < 0) break
                        if (received + n > LIMIT || (decision.total >= 0 && received + n > decision.total)) throw PermanentFailure("Invalid download length")
                        output.write(bytes, 0, n); received += n
                        if (System.currentTimeMillis() - checkpoint >= 1000) {
                            output.fd.sync(); update(id, work) { it.received = received }; checkpoint = System.currentTimeMillis()
                        }
                    }
                }
                output.fd.sync(); update(id, work) { it.received = received }
                if (decision.total >= 0 && received != decision.total) throw NetworkFailure("Download was interrupted")
            }
            finish(c, id, work, file, r.target)
        } finally { connection.disconnect(); work.connection = null }
    }
    private fun finish(c: Context, id: String, work: Work, file: File, target: String) {
        work.check(); RepoSources.validateIPA(file); work.check()
        synchronized(this) {
            val r = records.getValue(id)
            if (r.generation != work.generation) throw IOException("Superseded download")
            val output = File(root(c), target)
            if (output.exists()) {
                // A process death between rename and checkpoint may leave an already completed target.
                RepoSources.validateIPA(output)
                if (file.exists()) file.delete()
            } else if (!file.renameTo(output)) throw IOException("Cannot move IPA into library")
            r.status = "COMPLETE"; r.received = output.length(); r.total = output.length(); r.error = null
            android.util.Log.i("IpaDownloads", "Completed $id bytes=${r.received}")
            persist(); notifyChanged()
        }
    }
}

/** Pure HTTP resume policy, separately tested without weakening HTTPS enforcement. */
internal object DownloadProtocol {
    data class Decision(val offset: Long, val total: Long)
    fun restoreRecord(status: String, targetExists: Boolean): Boolean = status != "COMPLETE" || targetExists
    fun historyToRetire(records: List<Pair<String, String>>, capacity: Int): List<String> {
        val needed = (records.size - capacity + 1).coerceAtLeast(0)
        return records.asSequence().filter { it.second == "COMPLETE" }.take(needed).map { it.first }.toList()
    }
    fun retryDelay(attempt: Int): Long = (1000L shl attempt.coerceIn(0, 6)).coerceAtMost(60000L)
    fun validator(etag: String?, modified: String?): String? =
        etag?.takeIf { it.startsWith("\"") && it.endsWith("\"") && !it.startsWith("W/") && !it.contains('\n') && !it.contains('\r') }
            ?: modified?.takeIf { it.length <= 128 && Regex("[A-Za-z]{3}, [0-9]{2} [A-Za-z]{3} [0-9]{4} [0-9]{2}:[0-9]{2}:[0-9]{2} GMT").matches(it) }
    fun unsatisfiedLength(value: String?): Long? = value?.let { Regex("bytes \\*/([0-9]+)").matchEntire(it)?.groupValues?.get(1)?.toLongOrNull() }
    fun completeRange(offset: Long, total: Long?, prior: String?, current: String?): Boolean =
        offset > 0 && total == offset && prior != null && current == prior
    fun response(status: Int, offset: Long, range: String?, length: Long, prior: String?, validator: String?): Decision {
        if (status == 200) return Decision(0, length)
        require(status == 206) { "Unexpected HTTP response" }
        val match = Regex("bytes ([0-9]+)-([0-9]+)/([0-9]+)").matchEntire(range ?: "") ?: throw IOException("Invalid Content-Range")
        val start = match.groupValues[1].toLongOrNull() ?: throw IOException("Range overflow")
        val end = match.groupValues[2].toLongOrNull() ?: throw IOException("Range overflow")
        val total = match.groupValues[3].toLongOrNull() ?: throw IOException("Range overflow")
        if (start != offset || end < start || total <= end || end != total - 1 || (length >= 0 && length != end - start + 1)) throw IOException("Mismatched resume range")
        if (offset > 0 && (prior == null || validator != prior)) throw IOException("Resume validator changed")
        return Decision(offset, total)
    }
}
