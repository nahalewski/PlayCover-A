package org.touchhle.android

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/** Application-owned durable jobs; never retains an Activity or Apple credentials. */
object AppleStoreDownloadJobs {
    private val lock = Any()
    private val workers = Executors.newCachedThreadPool()
    private val running = ConcurrentHashMap.newKeySet<String>()
    private fun stateFile(c: Context) = File(c.noBackupFilesDir, "apple-store-download-jobs.json")
    private fun load(c: Context): JSONArray = if (stateFile(c).exists()) JSONArray(stateFile(c).readText()) else JSONArray()
    private fun save(c: Context, jobs: JSONArray) {
        val file = stateFile(c); val temp = File(file.parentFile, file.name + ".writing")
        FileOutputStream(temp).use { it.write(jobs.toString().toByteArray()); it.fd.sync() }
        require(temp.renameTo(file)) { "Could not persist download state" }
    }
    private fun update(c: Context, requestId: String, change: (JSONObject) -> Unit) = synchronized(lock) {
        val jobs = load(c)
        for (i in 0 until jobs.length()) if (jobs.getJSONObject(i).getString("request_id") == requestId) { change(jobs.getJSONObject(i)); save(c, jobs); return@synchronized }
    }
    fun descriptions(context: Context): List<String> = synchronized(lock) {
        val jobs = load(context); (0 until jobs.length()).map { val j=jobs.getJSONObject(it); "${j.getString("name")}: ${j.getString("status")}" }
    }
    fun enqueue(context: Context, appId: Long, name: String): String {
        require(appId > 0)
        val c = context.applicationContext
        val id = synchronized(lock) {
            val jobs = load(c)
            for (i in 0 until jobs.length()) {
                val j=jobs.getJSONObject(i)
                if (j.getLong("app_id")==appId && j.getString("status") !in listOf("complete","failed")) { start(c,j.getString("request_id")); return@synchronized j.getString("request_id") }
            }
            require(jobs.length()<128) { "Download job limit reached" }
            val key=UUID.randomUUID().toString().replace("-", "")
            jobs.put(JSONObject().put("request_id",key).put("app_id",appId).put("name",name).put("status","queued")); save(c,jobs); key
        }
        start(c,id); return id
    }
    fun resume(context: Context) {
        val c=context.applicationContext
        synchronized(lock) { val jobs=load(c); for (i in 0 until jobs.length()) {
            val j=jobs.getJSONObject(i); if (j.getString("status") !in listOf("complete","failed")) start(c,j.getString("request_id"))
        } }
    }
    private fun start(c: Context, id: String) {
        if (!running.add(id)) return
        workers.execute { try { execute(c,id) } catch (_: Exception) {
            // Network interruption remains resumable, with same idempotency key.
            update(c,id) { it.put("status","waiting for connection · resume when companion is available") }
        } finally { running.remove(id) } }
    }
    private fun connection(c: Context): Pair<String,String> {
        val config=JSONObject(File(c.noBackupFilesDir,"apple-store-companion.json").readText())
        val base=config.getString("url").trimEnd('/'); val uri=URI(base)
        require(uri.userInfo==null && uri.query==null && uri.fragment==null && uri.path.isNullOrEmpty())
        require(uri.scheme=="https" || (uri.scheme=="http" && uri.host in listOf("127.0.0.1","localhost","::1")))
        val token=config.getString("token"); require(token.length>=32); return base to token
    }
    private fun open(config: Pair<String,String>, path: String): HttpURLConnection = (URL(config.first+path).openConnection() as HttpURLConnection).apply {
        connectTimeout=15000; readTimeout=120000; instanceFollowRedirects=false; setRequestProperty("Authorization","Bearer "+config.second)
    }
    private fun json(config: Pair<String,String>, path: String, body: JSONObject?=null): JSONObject {
        val conn=open(config,path)
        try {
            if(body!=null){conn.requestMethod="POST"; conn.doOutput=true; conn.setRequestProperty("Content-Type","application/json"); conn.outputStream.use{it.write(body.toString().toByteArray())}}
            require(conn.responseCode in 200..299){"Companion unavailable"}
            return JSONObject(conn.inputStream.bufferedReader().use{it.readText()})
        } finally {conn.disconnect()}
    }
    private fun execute(c: Context, requestId: String) {
        val config=connection(c)
        val job=synchronized(lock){val jobs=load(c); (0 until jobs.length()).map{jobs.getJSONObject(it)}.first{it.getString("request_id")==requestId}}
        var serverId=job.optString("server_id")
        if(serverId.isEmpty()) {
            val made=json(config,"/v1/downloads",JSONObject().put("app_id",job.getLong("app_id")).put("request_id",requestId))
            serverId=made.getString("id"); require(serverId.matches(Regex("[0-9a-f]{32}")))
            update(c,requestId){it.put("server_id",serverId).put("status","downloading on computer")}
        }
        var completed: JSONObject?=null
        for (attempt in 0 until 1800) {
            val remote=json(config,"/v1/downloads/$serverId")
            if(remote.getString("status")=="complete"){completed=remote;break}
            if(remote.getString("status")=="failed"){update(c,requestId){it.put("status","failed")};return}
            update(c,requestId){it.put("status","${remote.optLong("bytes")/(1024*1024)} MiB on computer")}; Thread.sleep(2000)
        }
        val done=completed ?: throw IllegalStateException("Companion still downloading")
        val total=done.getLong("bytes"); val expected=done.getString("sha256"); require(expected.matches(Regex("[0-9a-f]{64}")))
        val dir=File(c.getExternalFilesDir(null),"touchHLE_apps").apply{mkdirs()}
        val part=File(dir,"$serverId.apple-store-part"); val target=File(dir,"${job.getLong("app_id") }-$serverId.ipa")
        val digest=MessageDigest.getInstance("SHA-256")
        fun hash(file: File) {file.inputStream().use{input->val buffer=ByteArray(256*1024); while(true){val n=input.read(buffer);if(n<0)break;digest.update(buffer,0,n)}}}
        if(target.exists()){hash(target); require(target.length()==total && hex(digest.digest())==expected); update(c,requestId){it.put("status","complete")};return}
        var offset=if(part.exists())part.length() else 0L; require(offset<=total)
        if(offset>0)hash(part)
        if(offset<total) {
            val conn=open(config,"/v1/files/$serverId")
            try {
                if(offset>0){conn.setRequestProperty("Range","bytes=$offset-");conn.setRequestProperty("If-Range","\"$expected\"")}
                val code=conn.responseCode
                if(code==200 && offset>0){offset=0;digest.reset()}
                else require(code==200 || (code==206 && conn.getHeaderField("Content-Range")=="bytes $offset-${total-1}/$total"))
                conn.inputStream.use{input->FileOutputStream(part,offset>0).use{output->
                    val buffer=ByteArray(256*1024);while(true){val n=input.read(buffer);if(n<0)break;require(offset+n<=total);output.write(buffer,0,n);digest.update(buffer,0,n);offset+=n};output.fd.sync()
                }}
            } finally {conn.disconnect()}
        }
        require(offset==total && hex(digest.digest())==expected){"IPA integrity check failed; partial preserved"}
        require(!target.exists() && part.renameTo(target));update(c,requestId){it.put("status","complete")}
    }
    private fun hex(bytes: ByteArray)=bytes.joinToString(""){"%02x".format(it.toInt() and 255)}
}
