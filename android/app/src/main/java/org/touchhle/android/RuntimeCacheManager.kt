package org.touchhle.android

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.io.BufferedInputStream
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors
import java.util.zip.ZipEntry
import java.util.zip.ZipInputStream

/**
 * Manages acquisition, extraction, and validation of the Apple iOS 16 dyld shared cache
 * required to link and execute 64-bit iOS applications.
 */
object RuntimeCacheManager {
    private const val TAG = "RuntimeCacheManager"

    const val MAIN_CACHE_FILENAME = "dyld_shared_cache_arm64"
    const val EXPECTED_CACHE_FILE_COUNT = 44
    const val PREF_CACHE_URL = "runtime_cache_custom_url"

    // Official Apple restore IPSW containing iOS 16.7.16 (20H392) Cryptex SystemOS cache for iPhone 8 (arm64 non-PAC)
    const val DEFAULT_IPSW_URL =
        "https://updates.cdn-apple.com/2024FallFCS/fullrestores/062-81729/7AE38B31-D42B-4509-940E-CD694939C24A/iPhone_4.7_P3_16.7.16_20H392_Restore.ipsw"

    private val executor = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    interface ProgressCallback {
        fun onProgress(message: String, progressPercent: Int)
        fun onSuccess(fileCount: Int)
        fun onError(error: String)
    }

    fun getCacheDir(context: Context): File {
        val root = context.getExternalFilesDir(null) ?: context.filesDir
        return File(root, "ios-runtime/cache").also { it.mkdirs() }
    }

    fun getMainCacheFile(context: Context): File {
        return File(getCacheDir(context), MAIN_CACHE_FILENAME)
    }

    fun isCachePresent(context: Context): Boolean {
        val file = getMainCacheFile(context)
        return file.exists() && file.length() > 0
    }

    fun countCacheFiles(context: Context): Int {
        val dir = getCacheDir(context)
        if (!dir.exists() || !dir.isDirectory) return 0
        return dir.listFiles { _, name -> name.startsWith(MAIN_CACHE_FILENAME) }?.size ?: 0
    }

    fun getCacheStatusDescription(context: Context): String {
        val count = countCacheFiles(context)
        return if (isCachePresent(context)) {
            "Installed ($count files ready)"
        } else {
            "Not installed (required for 64-bit apps)"
        }
    }

    fun downloadAndExtract(
        context: Context,
        sourceUrlString: String,
        callback: ProgressCallback
    ) {
        executor.execute {
            var connection: HttpURLConnection? = null
            try {
                val targetDir = getCacheDir(context)
                postProgress(callback, "Connecting to download server...", 0)

                var currentUrl = sourceUrlString
                var redirects = 0
                while (redirects < 5) {
                    val url = URL(currentUrl)
                    connection = (url.openConnection() as HttpURLConnection).apply {
                        instanceFollowRedirects = true
                        connectTimeout = 30_000
                        readTimeout = 60_000
                        setRequestProperty("User-Agent", "PlayCover-A/1.0 RuntimeDownloader")
                        setRequestProperty("Accept", "*/*")
                    }
                    val code = connection.responseCode
                    if (code in listOf(HttpURLConnection.HTTP_MOVED_PERM, HttpURLConnection.HTTP_MOVED_TEMP, HttpURLConnection.HTTP_SEE_OTHER, 307, 308)) {
                        val newUrl = connection.getHeaderField("Location")
                        connection.disconnect()
                        if (!newUrl.isNullOrBlank()) {
                            currentUrl = newUrl
                            redirects++
                            continue
                        }
                    }
                    break
                }

                val responseCode = connection!!.responseCode
                if (responseCode !in 200..299) {
                    throw IOException("HTTP error $responseCode: ${connection.responseMessage}")
                }

                val totalLength = connection.contentLengthLong
                val inputStream = BufferedInputStream(connection.inputStream, 64 * 1024)

                postProgress(callback, "Downloading and extracting archive...", 5)

                val zipIn = ZipInputStream(inputStream)
                var entry: ZipEntry? = try { zipIn.nextEntry } catch (_: Exception) { null }
                var extractedCount = 0
                val buffer = ByteArray(64 * 1024)
                var bytesReadTotal = 0L

                if (entry != null) {
                    while (entry != null) {
                        val name = entry.name
                        val fileName = File(name).name

                        if (fileName.startsWith(MAIN_CACHE_FILENAME) && !entry.isDirectory) {
                            val outFile = File(targetDir, fileName)
                            postProgress(
                                callback,
                                "Extracting $fileName...",
                                if (totalLength > 0) (bytesReadTotal * 100 / totalLength).toInt().coerceIn(10, 95) else 50
                            )

                            FileOutputStream(outFile).use { fos ->
                                var len: Int
                                while (zipIn.read(buffer).also { len = it } > 0) {
                                    fos.write(buffer, 0, len)
                                    bytesReadTotal += len
                                }
                            }
                            extractedCount++
                            Log.i(TAG, "Extracted cache entry: $fileName (${outFile.length()} bytes)")
                        } else {
                            var len: Int
                            while (zipIn.read(buffer).also { len = it } > 0) {
                                bytesReadTotal += len
                            }
                        }

                        zipIn.closeEntry()
                        entry = try { zipIn.nextEntry } catch (_: Exception) { null }
                    }
                } else {
                    // Direct file stream fallback if stream is not a ZIP archive
                    val outFile = getMainCacheFile(context)
                    FileOutputStream(outFile).use { fos ->
                        var len: Int
                        while (inputStream.read(buffer).also { len = it } > 0) {
                            fos.write(buffer, 0, len)
                            bytesReadTotal += len
                            postProgress(
                                callback,
                                "Downloading $MAIN_CACHE_FILENAME...",
                                if (totalLength > 0) (bytesReadTotal * 100 / totalLength).toInt().coerceIn(5, 95) else 50
                            )
                        }
                    }
                    if (outFile.exists() && outFile.length() > 0) {
                        extractedCount = 1
                    }
                }

                if (extractedCount == 0 && !isCachePresent(context)) {
                    throw IOException(
                        "No dyld_shared_cache_arm64 files were found in the downloaded archive."
                    )
                }

                val finalCount = countCacheFiles(context)
                postProgress(callback, "Cache ready ($finalCount files).", 100)
                mainHandler.post { callback.onSuccess(finalCount) }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to download and extract runtime cache", e)
                mainHandler.post { callback.onError(e.localizedMessage ?: "Unknown error") }
            } finally {
                connection?.disconnect()
            }
        }
    }

    private fun postProgress(callback: ProgressCallback, message: String, percent: Int) {
        mainHandler.post { callback.onProgress(message, percent) }
    }
}
