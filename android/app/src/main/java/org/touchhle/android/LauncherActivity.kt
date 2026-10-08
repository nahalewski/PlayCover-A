/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.app.Activity
import android.app.AlertDialog
import android.app.ProgressDialog
import android.content.Intent
import android.graphics.Color
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.text.format.Formatter
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.BaseAdapter
import android.widget.Button
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.ScrollView
import android.widget.ProgressBar
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import android.widget.EditText
import android.widget.ListView
import android.widget.ImageView
import android.widget.ArrayAdapter
import android.text.TextWatcher
import android.text.Editable
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors
import android.util.LruCache
import java.io.File

/**
 * The app's home screen: a library of the iOS apps (.ipa files) that have been
 * added, with a button to add more from anywhere in device storage using
 * Android's file picker. Tapping an app starts it in [MainActivity], which
 * hosts the emulator.
 */
class LauncherActivity : Activity() {
    private lateinit var appsDir: File
    private val apps = ArrayList<File>()
    private var downloadJobs: List<IpaDownloads.Job> = emptyList()
    private var launchMetadataPending = false
    private val downloadsChanged: () -> Unit = {
        if (!isFinishing && !isDestroyed && ::adapter.isInitialized) reload()
    }
    private lateinit var adapter: AppAdapter
    private lateinit var emptyView: View
    private lateinit var installedList: ListView
    private val repositoryWorker = Executors.newSingleThreadExecutor()
    private var repositoryTransfer: RepoSources.Transfer? = null
    private var repositoryProgress: ProgressDialog? = null
    private lateinit var libraryPage: View
    private var repositoryPage = "library"
    private var repositorySourceURL: String? = null
    private var repositoryQuery = ""
    private var repositoryCategory: RepoSources.Category? = null
    private val iconWorker = Executors.newFixedThreadPool(2)
    private val pendingIcons = HashMap<String, RepoSources.Transfer>()
    private val failedIcons = HashSet<String>()
    private val iconCache = object : LruCache<String, Bitmap>(12 * 1024) {
        override fun sizeOf(key: String, value: Bitmap): Int = (value.byteCount / 1024).coerceAtLeast(1)
    }

    private val prefs by lazy { getSharedPreferences("launcher", MODE_PRIVATE) }
    private val compat by lazy { CompatStore(prefs, getExternalFilesDir(null)!!) }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    private fun rounded(color: Int, radiusDp: Int): GradientDrawable =
        GradientDrawable().apply {
            setColor(color)
            cornerRadius = dp(radiusDp).toFloat()
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.statusBarColor = BACKGROUND
        window.navigationBarColor = BACKGROUND

        appsDir = File(getExternalFilesDir(null), "touchHLE_apps").also { it.mkdirs() }
        getExternalFilesDir(null)?.let { ScanCache.init(it) }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(BACKGROUND)
            fitsSystemWindows = true
        }

        val header = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(20), dp(24), dp(20), dp(12))
        }
        header.addView(TextView(this).apply {
            text = "Anastasis"; textSize = 30f; setTextColor(Color.WHITE)
            typeface = Typeface.create("sans-serif-medium", Typeface.BOLD)
        }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        header.addView(actionButton("＋ Add IPA") { pickIpa() })
        root.addView(header)
        root.addView(sectionLabel("INSTALLED IPAS"))
        val content = FrameLayout(this)
        adapter = AppAdapter()
        installedList = ListView(this).apply {
            divider = null; dividerHeight = dp(8)
            setPadding(dp(16), dp(4), dp(16), dp(16)); clipToPadding = false
            selector = rounded(Color.TRANSPARENT, 0)
            adapter = this@LauncherActivity.adapter
            setOnItemClickListener { _, _, position, _ ->
                apps.getOrNull(position - downloadJobs.size)?.let { launch(it) }
            }
            setOnItemLongClickListener { _, _, position, _ ->
                apps.getOrNull(position - downloadJobs.size)?.let { addHomeShortcut(it) }
                true
            }
        }
        content.addView(installedList)
        emptyView = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER
            setPadding(dp(32), dp(32), dp(32), dp(32))
            addView(TextView(context).apply {
                text = "No installed IPAs"; textSize = 22f; setTextColor(Color.WHITE)
                gravity = Gravity.CENTER
            })
            addView(TextView(context).apply {
                text = "Add an IPA from your files, or browse a repository. App compatibility depends on the emulator."
                textSize = 14f; setTextColor(TEXT_DIM); gravity = Gravity.CENTER
                setPadding(0, dp(12), 0, 0)
            })
        }
        content.addView(emptyView, FrameLayout.LayoutParams(-1, -1))
        root.addView(content, LinearLayout.LayoutParams(-1, 0, 1f))
        root.addView(bottomTabs("library"))

        libraryPage = root
        setContentView(libraryPage)
        reload()
        IpaDownloads.resume(this)
        if (HomeShortcuts.consumeIntent(this, intent, appsDir) { launch(it) }) {
            // Shortcut intent has been consumed by the validated launcher helper.
        } else if (intent?.action == Intent.ACTION_VIEW && intent?.data != null) {
            handleRepositoryIntent(intent)
        } else {
            repositoryCategory = savedInstanceState?.getString("repository_category")?.let { name ->
                runCatching { RepoSources.Category.valueOf(name) }.getOrNull()
            }
            when (savedInstanceState?.getString("repository_page")) {
                "sources" -> showRepositories()
                "settings" -> showSettings()
                "store" -> showAppleStore() 
                "apps" -> {
                    showRepositories()
                    repositoryQuery = savedInstanceState?.getString("repository_query", "") ?: ""
                    savedInstanceState?.getString("repository_url")?.let { fetchRepository(it) }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (!HomeShortcuts.consumeIntent(this, intent, appsDir) { launch(it) }) handleRepositoryIntent(intent)
    }

    override fun onDestroy() {
        IpaDownloads.removeListener(downloadsChanged)
        repositoryTransfer?.cancel()
        repositoryProgress?.dismiss()
        repositoryWorker.shutdownNow()
        pendingIcons.values.forEach { it.cancel() }
        iconWorker.shutdownNow()
        infoExecutor.shutdownNow()
        super.onDestroy()
    }

    private fun handleRepositoryIntent(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        val link = intent.dataString ?: return
        // Consume before presenting anything, so rotation never replays this link.
        intent.action = null
        intent.data = null
        setIntent(intent)
        try {
            val url = RepoSources.sourceURL(link)
            showRepositories()
            if (savedRepositories().any { it.url == url }) fetchRepository(url)
            else addRepository(url)
        }
        catch (error: Exception) { repositoryError(error) }
    }

    private fun savedRepositories(): List<RepoSources.Source> {
        return try {
            val array = JSONArray(prefs.getString("repositories", "[]"))
            (0 until array.length()).map { index ->
                val source = array.getJSONObject(index)
                RepoSources.Source(source.getString("name"), RepoSources.sourceURL(source.getString("url")))
            }
        } catch (_: Exception) { emptyList() }
    }

    private fun saveRepositories(sources: List<RepoSources.Source>) {
        val array = JSONArray()
        sources.forEach { array.put(JSONObject().put("name", it.name).put("url", it.url)) }
        prefs.edit().putString("repositories", array.toString()).apply()
    }

    private fun showRepositories() {
        repositoryPage = "sources"
        repositorySourceURL = null
        val sources = savedRepositories()
        val page = repositoryPageView("Repositories") { showLibrary() }
        page.addView(actionButton("Add source") { addRepository(RepoSources.EXAMPLE_SOURCE) })
        page.addView(actionButton("Games collection") {
            repositoryQuery = ""
            repositoryCategory = RepoSources.Category.GAMES
            fetchRepository("playcover:games")
        })
        if (sources.isEmpty()) page.addView(TextView(this).apply {
            text = "Add an HTTPS or AltStore source to browse apps."
            setPadding(dp(16), dp(16), dp(16), dp(16)); setTextColor(TEXT_DIM)
        })
        val list = ListView(this).apply {
            divider = null; dividerHeight = dp(8)
            adapter = object : ArrayAdapter<String>(this@LauncherActivity, android.R.layout.simple_list_item_1,
                sources.map { "${it.name}\n${it.url}" }) {
                override fun getView(position: Int, convertView: View?, parent: ViewGroup): View =
                    (super.getView(position, convertView, parent) as TextView).apply {
                        setTextColor(Color.WHITE); textSize = 15f; background = rounded(CARD, 14)
                        setPadding(dp(16), dp(18), dp(16), dp(18))
                    }
            }
            setOnItemClickListener { _, _, position, _ -> repositoryQuery = ""; fetchRepository(sources[position].url) }
            setOnItemLongClickListener { _, _, position, _ ->
                val source = sources[position]
                AlertDialog.Builder(this@LauncherActivity).setTitle("Remove ${source.name}?")
                    .setPositiveButton("Remove") { _, _ -> saveRepositories(sources.filter { it.url != source.url }); showRepositories() }
                    .setNegativeButton("Cancel", null).show()
                true
            }
        }
        page.addView(list, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        displayPage(page, "sources")
    }

    private fun repositoryPageView(title: String, back: () -> Unit): LinearLayout {
        val page = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setBackgroundColor(BACKGROUND); fitsSystemWindows = true
            setPadding(dp(16), dp(12), dp(16), dp(12))
        }
        page.addView(Button(this).apply {
            text = "‹ Back"; isAllCaps = false; setTextColor(ACCENT); background = rounded(CARD, 12); setOnClickListener { back() }
        }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(44)))
        page.addView(TextView(this).apply {
            text = title; setTextColor(Color.WHITE); textSize = 24f
            setPadding(dp(4), dp(12), dp(4), dp(16))
        })
        return page
    }

    private fun showLibrary() {
        repositoryTransfer?.cancel()
        repositoryPage = "library"
        repositorySourceURL = null
        setContentView(libraryPage)
        reload()
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        when (repositoryPage) {
            "apps" -> { repositoryTransfer?.cancel(); showRepositories() }
            "sources", "settings", "store" -> showLibrary()
            else -> super.onBackPressed()
        }
    }

    override fun onSaveInstanceState(state: Bundle) {
        state.putString("repository_page", repositoryPage)
        state.putString("repository_url", repositorySourceURL)
        state.putString("repository_query", repositoryQuery)
        state.putString("repository_category", repositoryCategory?.name)
        super.onSaveInstanceState(state)
    }

    private fun addRepository(initial: String) {
        val input = EditText(this).apply {
            setText(initial)
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_URI
            isSingleLine = true
            hint = "HTTPS source URL or AltStore source link"
        }
        val dialog = AlertDialog.Builder(this).setTitle("Add repository")
            .setView(input).setPositiveButton("Add", null).setNegativeButton("Cancel", null).create()
        dialog.setOnShowListener {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                try {
                    val url = RepoSources.sourceURL(input.text.toString())
                    dialog.dismiss()
                    fetchRepository(url)
                } catch (error: Exception) { input.error = error.message ?: "Invalid source URL" }
            }
        }
        dialog.show()
    }

    private fun repositoryError(error: Exception) {
        if (!isFinishing && !isDestroyed) AlertDialog.Builder(this)
            .setTitle("Repository error").setMessage(error.message ?: "Request failed")
            .setPositiveButton("OK", null).show()
    }

    private fun startRepositoryTransfer(title: String): RepoSources.Transfer? {
        if (repositoryTransfer != null) {
            Toast.makeText(this, "A repository transfer is already running", Toast.LENGTH_SHORT).show()
            return null
        }
        val transfer = RepoSources.Transfer()
        repositoryTransfer = transfer
        repositoryProgress = ProgressDialog(this).apply {
            setTitle(title); setMessage("Connecting…"); setCancelable(true)
            setOnCancelListener { transfer.cancel() }
            setButton(ProgressDialog.BUTTON_NEGATIVE, "Cancel") { _, _ -> transfer.cancel() }
            show()
        }
        return transfer
    }

    private fun finishRepositoryTransfer() {
        repositoryProgress?.dismiss()
        repositoryProgress = null
        repositoryTransfer = null
    }

    private fun fetchRepository(url: String) {
        val transfer = startRepositoryTransfer("Loading repository") ?: return
        repositoryWorker.execute {
            try {
                val feed = if (url == "playcover:games") {
                    assets.open("games.json").bufferedReader(Charsets.UTF_8).use { RepoSources.parseFeed(it.readText()) }
                } else RepoSources.fetch(url, transfer)
                transfer.check()
                runOnUiThread {
                    finishRepositoryTransfer()
                    if (!isFinishing && !isDestroyed && !transfer.cancelled.get()) {
                        if (url != "playcover:games") {
                            saveRepositories(savedRepositories().filter { it.url != url } + RepoSources.Source(feed.name, url))
                        }
                        browseRepository(feed, url)
                    }
                }
            } catch (error: Exception) { runOnUiThread {
                finishRepositoryTransfer()
                if (!transfer.cancelled.get()) repositoryError(error)
            } }
        }
    }

    private fun browseRepository(feed: RepoSources.Feed, url: String) {
        repositoryPage = "apps"
        repositorySourceURL = url
        val layout = repositoryPageView(feed.name) { showRepositories() }
        val search = EditText(this).apply {
            hint = "Search apps"; isSingleLine = true; setTextColor(Color.WHITE); setHintTextColor(TEXT_DIM)
            background = rounded(CARD, 14); setPadding(dp(16), dp(12), dp(16), dp(12))
        }
        layout.addView(search)
        if (feed.skipped > 0) layout.addView(TextView(this).apply {
            text = "${feed.skipped} listings have no supported HTTPS download."
        })
        val visible = ArrayList(feed.apps)
        val rows = RepositoryAppAdapter(visible)
        val filters = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(8), 0, dp(8))
        }
        val filterButtons = ArrayList<Pair<RepoSources.Category?, Button>>()
        fun refreshResults() {
            val query = repositoryQuery.trim()
            visible.clear()
            visible.addAll(feed.apps.filter { app ->
                (repositoryCategory == null || app.category == repositoryCategory) &&
                    (app.name.contains(query, true) || app.identifier.contains(query, true))
            })
            rows.notifyDataSetChanged()
            filterButtons.forEach { (category, button) ->
                val selected = category == repositoryCategory
                button.isSelected = selected
                button.setTextColor(if (selected) Color.BLACK else Color.WHITE)
                button.background = rounded(if (selected) ACCENT else CARD, 18)
                button.contentDescription = "${button.text} filter${if (selected) ", selected" else ""}"
            }
        }
        val categories: List<RepoSources.Category?> = listOf(null) + RepoSources.Category.values().toList()
        categories.forEach { category ->
            val button = Button(this).apply {
                text = category?.label ?: "All"
                isAllCaps = false
                textSize = 14f
                minWidth = 0
                minimumWidth = 0
                setPadding(dp(16), 0, dp(16), 0)
                setOnClickListener {
                    repositoryCategory = category
                    refreshResults()
                }
            }
            filterButtons.add(category to button)
            filters.addView(button, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(44)).apply {
                marginEnd = dp(8)
            })
        }
        layout.addView(HorizontalScrollView(this).apply {
            isHorizontalScrollBarEnabled = false
            addView(filters)
        })
        val list = ListView(this).apply { adapter = rows; divider = null; dividerHeight = dp(8) }
        val noResults = TextView(this).apply {
            text = "No apps match these filters."
            setTextColor(TEXT_DIM)
            setPadding(dp(8), dp(12), dp(8), dp(12))
        }
        layout.addView(noResults)
        layout.addView(list, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        list.emptyView = noResults
        search.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                val query = s.toString().trim()
                repositoryQuery = query
                refreshResults()
            }
            override fun afterTextChanged(s: Editable?) {}
        })
        list.setOnItemClickListener { _, _, position, _ ->
            val app = visible[position]
            AlertDialog.Builder(this).setTitle(app.name)
                .setMessage("Version ${app.version}\n${app.identifier}\n${app.downloadURL}\n\nDownload the IPA to your library. Compatibility depends on the app.")
                .setPositiveButton("Download") { _, _ -> downloadRepositoryApp(app) }
                .setNegativeButton("Cancel", null).show()
        }
        search.setText(repositoryQuery)
        refreshResults()
        displayPage(layout, "sources")
    }

    private inner class RepositoryAppAdapter(private val rows: List<RepoSources.App>) : BaseAdapter() {
        override fun getCount() = rows.size
        override fun getItem(position: Int) = rows[position]
        override fun getItemId(position: Int) = position.toLong()
        override fun getView(position: Int, convertView: View?, parent: ViewGroup?): View {
            val row = convertView as? LinearLayout ?: LinearLayout(this@LauncherActivity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                setPadding(dp(12), dp(12), dp(12), dp(12)); background = rounded(CARD, 14)
                addView(ImageView(this@LauncherActivity).apply { scaleType = ImageView.ScaleType.CENTER_CROP },
                    LinearLayout.LayoutParams(dp(56), dp(56)))
                addView(TextView(this@LauncherActivity).apply {
                    id = android.R.id.text1; setTextColor(Color.WHITE); textSize = 16f
                    setPadding(dp(14), 0, 0, 0)
                }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            }
            val app = rows[position]
            (row.getChildAt(1) as TextView).text = "${app.name} · ${app.version}\n${app.developer}"
            val image = row.getChildAt(0) as ImageView
            image.tag = app.iconURL
            image.contentDescription = "${app.name} icon placeholder"
            image.setImageDrawable(rounded(CARD, 12))
            val url = app.iconURL
            if (url != null) {
                val bitmap = iconCache.get(url)
                if (bitmap != null) {
                    image.setImageBitmap(bitmap)
                    image.contentDescription = "${app.name} icon loaded"
                } else if (!failedIcons.contains(url) && !pendingIcons.containsKey(url) && pendingIcons.size < 32) {
                    val transfer = RepoSources.Transfer()
                    pendingIcons[url] = transfer
                    iconWorker.execute {
                        val decoded = try {
                            val bytes = RepoSources.fetchIcon(url, transfer)
                            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                            require(bounds.outWidth > 0 && bounds.outHeight > 0 &&
                                bounds.outWidth.toLong() * bounds.outHeight <= 16_000_000)
                            val options = BitmapFactory.Options().apply { inSampleSize = 1 }
                            while (bounds.outWidth / options.inSampleSize > 256 || bounds.outHeight / options.inSampleSize > 256) options.inSampleSize *= 2
                            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
                        } catch (_: Exception) { null } catch (_: OutOfMemoryError) { null }
                        runOnUiThread {
                            pendingIcons.remove(url)
                            if (!isFinishing && !isDestroyed && !transfer.cancelled.get()) {
                                if (decoded != null) iconCache.put(url, decoded) else failedIcons.add(url)
                                // Rebinding checks each recycled row's URL; no stale image assignment.
                                notifyDataSetChanged()
                                this@LauncherActivity.adapter.notifyDataSetChanged()
                            }
                        }
                    }
                }
            }
            return row
        }
    }

    private fun downloadRepositoryApp(app: RepoSources.App) {
        try {
            IpaDownloads.enqueue(this, app)
            showLibrary()
        } catch (error: Exception) { repositoryError(error) }
    }

    override fun onStart() {
        super.onStart()
        IpaDownloads.addListener(downloadsChanged)
        if (::adapter.isInitialized) reload()
    }

    override fun onStop() {
        IpaDownloads.removeListener(downloadsChanged)
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        reload()
        checkForCrashReport()
    }

    private fun reload() {
        downloadJobs = IpaDownloads.snapshot(this).filter { it.status != "COMPLETE" }
        apps.clear()
        appsDir.listFiles()
            ?.filter { it.isFile && it.name.endsWith(".ipa", ignoreCase = true) ||
                it.isDirectory && it.name.endsWith(".app", ignoreCase = true) }
            ?.sortedBy { it.name.lowercase() }
            ?.let { apps.addAll(it) }
        adapter.notifyDataSetChanged()
        val empty = apps.isEmpty() && downloadJobs.isEmpty()
        emptyView.visibility = if (empty) View.VISIBLE else View.GONE
        installedList.visibility = if (empty) View.GONE else View.VISIBLE
    }

    // --- Crash report ------------------------------------------------------

    /**
     * If the last game ended in a Rust panic (the emulator writes it to
     * touchHLE_log.txt), explain what happened instead of silently dropping
     * back to the library.
     */
    private fun checkForCrashReport() {
        val log = File(getExternalFilesDir(null), "touchHLE_log.txt")
        if (!log.isFile) return
        val stamp = log.lastModified()
        val launchedAt = prefs.getLong("last_launch_time", 0L)
        if (stamp <= prefs.getLong("crash_seen_ts", 0L) || stamp + 5000 < launchedAt) return
        val lines = try { log.readLines() } catch (_: Exception) { return }
        val panicAt = lines.indexOfFirst { it.startsWith("Panic at") }
        if (panicAt < 0) return
        if (runningGameProcesses().isNotEmpty()) return // still running: not a crash we can report yet
        prefs.edit().putLong("crash_seen_ts", stamp).apply()

        val gameFile = prefs.getString("last_launch_name", null)
        val game = gameFile?.let { prettyName(File(it)) } ?: "The game"
        val panic = lines[panicAt]
        val guest = lines.firstOrNull { it.contains("Guest read error") || it.contains("Guest write error") }
        val registers = lines.drop(panicAt).dropWhile { !it.startsWith("Dumping registers") }.take(5)
        val stack = lines.drop(panicAt).dropWhile { !it.startsWith("Attempting to produce stack trace") }.take(10)
        val report = buildString {
            append("$game stopped: ").append(panic.removePrefix("Panic at ")).append('\n')
            if (guest != null) append('\n').append(guest.substringAfter("touchHLE::cpu: ")).append('\n')
            if (registers.isNotEmpty()) append('\n').append(registers.joinToString("\n")).append('\n')
            if (stack.isNotEmpty()) append('\n').append(stack.joinToString("\n")).append('\n')
        }.trim()
        if (gameFile != null) compat.recordCrash(gameFile, panic.removePrefix("Panic at ").take(160))
        adapter.notifyDataSetChanged()

        val body = TextView(this).apply {
            text = report; textSize = 12f; setTextColor(0xFFE6E0E9.toInt())
            typeface = Typeface.MONOSPACE; setTextIsSelectable(true)
            setPadding(dp(20), dp(8), dp(20), dp(8))
        }
        val dialog = AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("$game stopped unexpectedly")
            .setView(ScrollView(this).apply { addView(body) })
            .setPositiveButton("Copy report") { _, _ ->
                val clipboard = getSystemService(CLIPBOARD_SERVICE) as android.content.ClipboardManager
                clipboard.setPrimaryClip(android.content.ClipData.newPlainText("Anastasis crash report", report))
                android.widget.Toast.makeText(this, "Report copied", android.widget.Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton("Dismiss", null)
            .create()
        dialog.window?.setBackgroundDrawable(
            rounded(android.graphics.Color.argb(0xE6, 0x2B, 0x29, 0x30), 28))
        dialog.show()
    }

    // --- Compatibility details --------------------------------------------

    private fun showCompatibilityDetails(app: File) {
        val info = infoFor(app)
        val badge = compat.badge(app.name)
        val text = buildString {
            append("Status: ").append(badge.status.label).append(if (badge.manual) " (marked by you)" else "").append('\n')
            badge.note?.let { append(it).append('\n') }
            val scan = info?.scan
            if (scan == null) {
                append(if (info?.is64Bit == true) "\n64-bit apps are not checked here.\n" else "\nImport check not available for this app.\n")
            } else {
                append("\nImport check against this emulator build:\n")
                append("• ${scan.missingFunctions.size} missing C functions\n")
                append("• ${scan.missingConstants.size} missing constants\n")
                append("• ${scan.missingClasses.size} missing classes (usually tolerated)\n")
                val sample = (scan.missingFunctions + scan.missingConstants).take(14)
                if (sample.isNotEmpty()) append("\n").append(sample.joinToString("\n") { "   $it" })
                if (scan.serious > sample.size) append("\n   …and ${scan.serious - sample.size} more")
            }
        }
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(info?.displayName ?: prettyName(app))
            .setMessage(text)
            .setPositiveButton("Mark…") { _, _ -> markCompatibility(app) }
            .setNegativeButton("Close", null)
            .show()
    }

    private fun markCompatibility(app: File) {
        val options = listOf("Works" to CompatStore.Status.WORKS, "Partial" to CompatStore.Status.PARTIAL,
            "Broken" to CompatStore.Status.BROKEN, "Clear my mark" to null)
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Mark ${prettyName(app)} as")
            .setItems(options.map { it.first }.toTypedArray()) { _, which ->
                compat.mark(app.name, options[which].second)
                adapter.notifyDataSetChanged()
            }.show()
    }

    /** Processes of the emulator (the ":game" process) that are currently alive. */
    private fun runningGameProcesses(): List<Int> {
        val manager = getSystemService(ACTIVITY_SERVICE) as android.app.ActivityManager
        return manager.runningAppProcesses
            ?.filter { it.processName == "$packageName:game" }
            ?.map { it.pid }
            ?: emptyList()
    }

    private fun startGame(app: File) {
        val key = app.absolutePath + "@" + app.lastModified()
        if (app.isFile && !infoCache.containsKey(key)) {
            if (launchMetadataPending) return
            launchMetadataPending = true
            infoExecutor.execute {
                val info = IpaInfo.read(app)
                runOnUiThread {
                    launchMetadataPending = false
                    if (!isFinishing && !isDestroyed) {
                        infoCache[key] = info
                        adapter.notifyDataSetChanged()
                        startGame(app)
                    }
                }
            }
            return
        }
        compat.clearCrash(app.name)
        prefs.edit().putString("last_launch_name", app.name).putLong("last_launch_time", System.currentTimeMillis()).apply()
        val runtimeOptions = currentEmulatorSettings().runtimeArguments().toMutableList()
        infoCache[key]?.minimumIosVersion?.let { runtimeOptions.add("--reported-ios-version=$it") }
        startActivity(Intent(this, MainActivity::class.java).apply {
            putExtra(MainActivity.EXTRA_APP_PATH, app.absolutePath)
            putExtra(MainActivity.EXTRA_COMPAT, prefs.getBoolean(PREF_COMPAT, true))
            putExtra(MainActivity.EXTRA_RUNTIME_OPTIONS, runtimeOptions.toTypedArray())
        })
    }

    private fun launch(app: File) {
        val running = runningGameProcesses()
        if (running.isEmpty()) {
            startGame(app)
            return
        }
        // The emulator can only run one app per process, so a game that is
        // already running has to be quit before another one can start.
        AlertDialog.Builder(this)
            .setTitle("A game is already running")
            .setMessage("Switch back to it, or quit it and start ${prettyName(app)}?")
            .setPositiveButton("Switch back") { _, _ ->
                startActivity(
                    Intent(this, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)
                )
            }
            .setNegativeButton("Quit it and start") { _, _ ->
                running.forEach { android.os.Process.killProcess(it) }
                // Give Android a moment to tear the old process down.
                window.decorView.postDelayed({ startGame(app) }, 500)
            }
            .setNeutralButton("Cancel", null)
            .show()
    }

    private fun confirmRemove(app: File) {
        AlertDialog.Builder(this)
            .setTitle("Remove ${prettyName(app)}?")
            .setMessage("This deletes the file from Anastasis's storage. The original you picked is not touched.")
            .setPositiveButton("Remove") { _, _ ->
                if (app.isDirectory) app.deleteRecursively() else app.delete()
                reload()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    // --- Adding IPAs from device storage ---------------------------------

    private fun pickIpa() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        }
        @Suppress("DEPRECATION")
        startActivityForResult(intent, REQUEST_PICK_IPA)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_PICK_IPA || resultCode != RESULT_OK || data == null) return
        val uris = ArrayList<Uri>()
        data.clipData?.let { clip ->
            for (i in 0 until clip.itemCount) uris.add(clip.getItemAt(i).uri)
        }
        data.data?.let { if (uris.isEmpty()) uris.add(it) }
        importUris(uris)
    }

    private fun displayName(uri: Uri): String? =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }

    private fun safeName(name: String): String = name.replace(Regex("[\\\\/:*?\"<>|]"), "_")

    private fun uniqueFile(name: String): File {
        var file = File(appsDir, safeName(name))
        var n = 2
        while (file.exists()) {
            val base = name.substringBeforeLast('.')
            val ext = name.substringAfterLast('.', "")
            file = File(appsDir, safeName("$base ($n).$ext"))
            n++
        }
        return file
    }

    private fun importUris(uris: List<Uri>) {
        @Suppress("DEPRECATION")
        val progress = ProgressDialog(this).apply {
            setTitle("Adding apps")
            setMessage("Copying…")
            isIndeterminate = true
            setCancelable(false)
            show()
        }
        Thread {
            val added = ArrayList<String>()
            val skipped = ArrayList<String>()
            for (uri in uris) {
                val name = displayName(uri) ?: "app.ipa"
                if (!name.endsWith(".ipa", ignoreCase = true)) {
                    skipped.add(name)
                    continue
                }
                runOnUiThread { progress.setMessage("Copying $name…") }
                val dest = uniqueFile(name)
                try {
                    contentResolver.openInputStream(uri)?.use { input ->
                        dest.outputStream().use { output -> input.copyTo(output, 1 shl 20) }
                    } ?: throw java.io.IOException("could not open file")
                    added.add(name)
                } catch (e: Exception) {
                    dest.delete()
                    skipped.add("$name (${e.message})")
                }
            }
            runOnUiThread {
                progress.dismiss()
                reload()
                val message = buildString {
                    if (added.isNotEmpty()) append("Added ${added.size} app${if (added.size == 1) "" else "s"}")
                    if (skipped.isNotEmpty()) {
                        if (isNotEmpty()) append(". ")
                        append("Skipped: ${skipped.joinToString()}")
                    }
                }
                if (message.isNotEmpty()) Toast.makeText(this, message, Toast.LENGTH_LONG).show()
            }
        }.start()
    }

    // --- The tiles --------------------------------------------------------

    private fun prettyName(app: File): String =
        app.name.removeSuffix(".ipa").removeSuffix(".IPA").removeSuffix(".app")
            .replace('_', ' ').trim()

    /** Name and icon read from inside each .ipa, loaded in the background. */
    private val infoCache = HashMap<String, IpaInfo?>()
    private val infoLoading = HashSet<String>()
    private val infoExecutor = java.util.concurrent.Executors.newSingleThreadExecutor()

    private fun infoFor(app: File): IpaInfo? {
        val key = app.absolutePath + "@" + app.lastModified()
        if (infoCache.containsKey(key)) return infoCache[key]
        if (app.isFile && infoLoading.add(key)) {
            infoExecutor.execute {
                val info = IpaInfo.read(app)
                runOnUiThread {
                    if (!isFinishing && !isDestroyed) {
                        infoCache[key] = info
                        adapter.notifyDataSetChanged()
                    }
                }
            }
        }
        return null
    }

    private fun installedActions(app: File) {
        AlertDialog.Builder(this).setTitle(infoFor(app)?.displayName ?: prettyName(app))
            .setItems(arrayOf("Launch", "Compatibility details", "Add to home screen", "Remove")) { _, which ->
                when (which) { 0 -> launch(app); 1 -> showCompatibilityDetails(app); 2 -> addHomeShortcut(app); 3 -> confirmRemove(app) }
            }.setNegativeButton("Cancel", null).show()
    }

    private fun addHomeShortcut(app: File) {
        HomeShortcuts.pin(this, app, prettyName(app), infoFor(app)?.icon)
    }

    private fun actionButton(label: String, action: () -> Unit): Button = Button(this).apply {
        text = label; isAllCaps = false; setTextColor(Color.WHITE)
        background = rounded(ACCENT, 14); setPadding(dp(16), dp(8), dp(16), dp(8))
        stateListAnimator = null; setOnClickListener { action() }
    }

    private fun sectionLabel(label: String): TextView = TextView(this).apply {
        text = label; textSize = 12f; setTextColor(TEXT_DIM)
        setPadding(dp(20), dp(12), dp(20), dp(10))
    }

    private fun bottomTabs(selected: String): View = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL; background = rounded(CARD, 0)
        setPadding(dp(4), dp(8), dp(4), dp(8))
        listOf("library" to "Installed IPAs", "sources" to "Repositories", "store" to "App Store", "settings" to "Settings").forEach { (key, label) ->
            addView(TextView(this@LauncherActivity).apply {
                text = label; textSize = 13f; gravity = Gravity.CENTER; minHeight = dp(48)
                setTextColor(if (key == selected) ACCENT else TEXT_DIM)
                typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                setOnClickListener {
                    when (key) { "library" -> showLibrary(); "sources" -> showRepositories(); "store" -> showAppleStore(); else -> showSettings() }
                }
            }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        }
    }

    private fun displayPage(page: LinearLayout, selected: String) {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setBackgroundColor(BACKGROUND); fitsSystemWindows = true
        }
        root.addView(page, LinearLayout.LayoutParams(-1, 0, 1f))
        root.addView(bottomTabs(selected))
        setContentView(root)
    }


    private fun showAppleStore() {
        repositoryTransfer?.cancel()
        startActivity(Intent(this, AppleStoreActivity::class.java))
    }

    private fun showSettings() {
        repositoryTransfer?.cancel(); repositoryPage = "settings"; repositorySourceURL = null
        val page = repositoryPageView("Settings") { showLibrary() }
        val content = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        page.addView(ScrollView(this).apply { addView(content) },
            LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        content.addView(TextView(this).apply {
            text = "Changes apply the next time you launch an app. Default keeps each app's own settings. Android always uses full screen."
            setTextColor(TEXT_DIM); textSize = 13f
            setPadding(dp(4), 0, dp(4), dp(12))
        })
        content.addView(sectionLabel("APP STORE"))
        content.addView(actionButton("Apple account and connection") {
            startActivity(Intent(this, AppleStoreActivity::class.java).putExtra("settings", true))
        })
        content.addView(sectionLabel("DISPLAY"))
        addSettingChoice(content, "Internal resolution", "Higher resolutions may reduce performance or break some apps.",
            EmulatorSettings.SCALE, listOf("Default" to "default", "1×" to "1", "2×" to "2", "3×" to "3", "4×" to "4"))
        addSettingChoice(content, "Startup orientation", "Use an override when an app does not select its orientation correctly.",
            EmulatorSettings.ORIENTATION, listOf("Default" to "default", "Upside down" to "upside-down",
                "Landscape left" to "landscape-left", "Landscape right" to "landscape-right"))
        content.addView(sectionLabel("INPUT AND NETWORK"))
        addSettingSwitch(content, "Controller tilt controls", "Use a controller's analog sticks for simulated device tilt. Disable to use the device accelerometer.",
            EmulatorSettings.CONTROLLER_TILT, true)
        addSettingSwitch(content, "Allow network access", "Allow the running iOS app to access the internet and local network.",
            EmulatorSettings.NETWORK, false)
        content.addView(sectionLabel("EMULATION"))
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            background = rounded(CARD, 14); setPadding(dp(16), dp(16), dp(16), dp(16))
        }
        row.addView(TextView(this).apply {
            text = "Compatibility mode\nContinue past unsupported calls. This can hide app errors and does not guarantee compatibility."
            textSize = 14f; setTextColor(Color.WHITE)
        }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        row.addView(Switch(this).apply {
            contentDescription = "Compatibility mode"; isChecked = prefs.getBoolean(PREF_COMPAT, true)
            setOnCheckedChangeListener { _, checked -> prefs.edit().putBoolean(PREF_COMPAT, checked).apply() }
        })
        content.addView(row)
        content.addView(sectionLabel("Anastasis · powered by touchHLE"))
        displayPage(page, "settings")
    }

    private fun currentEmulatorSettings() = EmulatorSettings(
        prefs.getString(EmulatorSettings.SCALE, "default") ?: "default",
        prefs.getString(EmulatorSettings.ORIENTATION, "default") ?: "default",
        prefs.getBoolean(EmulatorSettings.CONTROLLER_TILT, true),
        prefs.getBoolean(EmulatorSettings.NETWORK, false)
    )

    private fun addSettingChoice(parent: LinearLayout, title: String, detail: String,
                                 key: String, choices: List<Pair<String, String>>) {
        val button = Button(this).apply {
            isAllCaps = false; gravity = Gravity.START or Gravity.CENTER_VERTICAL
            setTextColor(Color.WHITE); textSize = 14f
            setPadding(dp(16), dp(12), dp(16), dp(12)); background = rounded(CARD, 14)
        }
        fun updateLabel() {
            val choice = choices.firstOrNull { it.second == prefs.getString(key, "default") } ?: choices.first()
            button.text = "$title: ${choice.first}\n$detail"
            button.contentDescription = "$title, ${choice.first}. $detail"
        }
        updateLabel()
        button.setOnClickListener {
            val selected = choices.indexOfFirst { it.second == prefs.getString(key, "default") }.coerceAtLeast(0)
            AlertDialog.Builder(this).setTitle(title)
                .setSingleChoiceItems(choices.map { it.first }.toTypedArray(), selected) { dialog, which ->
                    prefs.edit().putString(key, choices[which].second).apply()
                    updateLabel(); dialog.dismiss()
                }.setNegativeButton("Cancel", null).show()
        }
        parent.addView(button, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(8) })
    }

    private fun addSettingSwitch(parent: LinearLayout, title: String, detail: String,
                                 key: String, default: Boolean) {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            background = rounded(CARD, 14); setPadding(dp(16), dp(16), dp(16), dp(16))
        }
        row.addView(TextView(this).apply {
            text = "$title\n$detail"; textSize = 14f; setTextColor(Color.WHITE)
        }, LinearLayout.LayoutParams(0, -2, 1f))
        row.addView(Switch(this).apply {
            contentDescription = title; isChecked = prefs.getBoolean(key, default)
            setOnCheckedChangeListener { _, checked -> prefs.edit().putBoolean(key, checked).apply() }
        })
        parent.addView(row, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(8) })
    }

    private inner class AppAdapter : BaseAdapter() {
        override fun getCount() = downloadJobs.size + apps.size
        override fun getItem(position: Int): Any = if (position < downloadJobs.size) downloadJobs[position]
            else apps[position - downloadJobs.size]
        override fun getItemId(position: Int) = position.toLong()
        override fun getViewTypeCount() = 2
        override fun getItemViewType(position: Int) = if (position < downloadJobs.size) 1 else 0
        override fun getView(position: Int, convertView: View?, parent: ViewGroup?): View {
            if (position < downloadJobs.size) return downloadRow(downloadJobs[position], convertView)
            val app = apps[position - downloadJobs.size]
            val info = infoFor(app)
            val name = info?.displayName ?: prettyName(app)
            val row = convertView as? LinearLayout ?: LinearLayout(this@LauncherActivity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                background = rounded(CARD, 16); setPadding(dp(14), dp(14), dp(14), dp(14))
                addView(FrameLayout(this@LauncherActivity).apply {
                    background = rounded(ACCENT, 13); clipToOutline = true
                    addView(TextView(context).apply {
                        gravity = Gravity.CENTER; setTextColor(Color.WHITE); textSize = 28f
                    }, FrameLayout.LayoutParams(-1, -1))
                    addView(ImageView(context).apply { scaleType = ImageView.ScaleType.CENTER_CROP }, FrameLayout.LayoutParams(-1, -1))
                }, LinearLayout.LayoutParams(dp(60), dp(60)))
                addView(LinearLayout(this@LauncherActivity).apply {
                    orientation = LinearLayout.VERTICAL; setPadding(dp(14), 0, dp(8), 0)
                    addView(TextView(context).apply {
                        id = android.R.id.text1; textSize = 17f; setTextColor(Color.WHITE); maxLines = 2
                        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                    })
                    addView(TextView(context).apply {
                        textSize = 12f; setTextColor(TEXT_DIM); maxLines = 2
                        ellipsize = android.text.TextUtils.TruncateAt.END; setPadding(0, dp(5), 0, 0)
                    })
                    addView(TextView(context).apply {
                        textSize = 11f; setTextColor(Color.WHITE); gravity = Gravity.CENTER
                        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                        setPadding(dp(10), dp(2), dp(10), dp(3))
                    }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(6) })
                }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
                addView(TextView(this@LauncherActivity).apply {
                    text = "⋮"; textSize = 28f; setTextColor(ACCENT)
                    gravity = Gravity.CENTER; isFocusable = true; isClickable = true
                    background = rounded(CARD, 12)
                }, LinearLayout.LayoutParams(dp(44), dp(48)))
            }
            // Explicit row handlers keep tap/hold usable with a focusable action
            // button, which otherwise suppresses ListView item clicks.
            row.isFocusable = true
            row.descendantFocusability = ViewGroup.FOCUS_BEFORE_DESCENDANTS
            row.setOnClickListener { launch(app) }
            row.setOnLongClickListener { addHomeShortcut(app); true }
            (row.getChildAt(2) as TextView).apply {
                contentDescription = "$name actions"
                setOnClickListener { installedActions(app) }
            }
            val iconFrame = row.getChildAt(0) as FrameLayout
            (iconFrame.getChildAt(0) as TextView).text = name.firstOrNull { it.isLetterOrDigit() }?.uppercase() ?: "?"
            (iconFrame.getChildAt(1) as ImageView).apply {
                setImageBitmap(info?.icon); visibility = if (info?.icon != null) View.VISIBLE else View.GONE
                contentDescription = "$name icon"
            }
            val labels = row.getChildAt(1) as LinearLayout
            (labels.getChildAt(0) as TextView).text = name
            val detail = if (app.isFile) Formatter.formatShortFileSize(this@LauncherActivity, app.length()) else "App folder"
            val requirement = info?.minimumIosVersion?.let { "Requires iOS $it" }
            val architecture = when (info?.is64Bit) {
                true -> "64-bit"
                false -> "32-bit"
                null -> "Architecture unknown"
            }
            val experimental = if (info?.is64Bit == true) "compatibility experimental" else null
            val scan = info?.scan
            val missing = scan?.let { if (it.serious == 0) "no missing imports" else "${it.serious} imports missing" }
            val metadata = listOfNotNull(architecture, requirement, experimental, missing).joinToString(" • ")
            (labels.getChildAt(1) as TextView).apply {
                maxLines = 3
                text = "${app.name}\n$detail" + if (metadata.isNotEmpty()) "\n$metadata" else ""
            }
            val badge = compat.badge(app.name)
            (labels.getChildAt(2) as TextView).apply {
                text = badge.status.label + if (badge.manual) " ✎" else ""
                background = rounded(badge.status.color, 10)
                contentDescription = "Compatibility: ${badge.status.label}" + (badge.note?.let { ". $it" } ?: "")
                setOnClickListener { showCompatibilityDetails(app) }
            }
            return row
        }

        private fun downloadRow(job: IpaDownloads.Job, convertView: View?): View {
            val row = convertView as? LinearLayout ?: LinearLayout(this@LauncherActivity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                background = rounded(CARD, 16); setPadding(dp(14), dp(14), dp(14), dp(14))
                addView(FrameLayout(context).apply {
                    background = rounded(ACCENT, 13); clipToOutline = true
                    addView(TextView(context).apply {
                        gravity = Gravity.CENTER; setTextColor(Color.WHITE); textSize = 28f
                    }, FrameLayout.LayoutParams(-1, -1))
                    addView(ImageView(context).apply { scaleType = ImageView.ScaleType.CENTER_CROP },
                        FrameLayout.LayoutParams(-1, -1))
                }, LinearLayout.LayoutParams(dp(60), dp(60)))
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL; setPadding(dp(14), 0, dp(8), 0)
                    addView(TextView(context).apply {
                        setTextColor(Color.WHITE); textSize = 17f; maxLines = 2
                        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                    })
                    addView(TextView(context).apply {
                        setTextColor(TEXT_DIM); textSize = 12f; maxLines = 3
                        setPadding(0, dp(4), 0, dp(6))
                    })
                    addView(ProgressBar(context, null, android.R.attr.progressBarStyleHorizontal).apply {
                        max = 1000
                        progressTintList = android.content.res.ColorStateList.valueOf(ACCENT)
                        indeterminateTintList = android.content.res.ColorStateList.valueOf(ACCENT)
                    }, LinearLayout.LayoutParams(-1, dp(12)))
                }, LinearLayout.LayoutParams(0, -2, 1f))
                addView(TextView(context).apply {
                    text = "⋮"; textSize = 28f; setTextColor(ACCENT); gravity = Gravity.CENTER
                    isClickable = true; isFocusable = true
                }, LinearLayout.LayoutParams(dp(44), dp(48)))
            }
            row.contentDescription = "${job.name} download"
            row.isClickable = false
            row.isLongClickable = false
            row.isFocusable = false
            val frame = row.getChildAt(0) as FrameLayout
            (frame.getChildAt(0) as TextView).text = job.name.firstOrNull { it.isLetterOrDigit() }?.uppercase() ?: "?"
            val image = frame.getChildAt(1) as ImageView
            bindDownloadIcon(image, job.iconURL, job.name)
            val labels = row.getChildAt(1) as LinearLayout
            (labels.getChildAt(0) as TextView).text = job.name
            val bytes = Formatter.formatFileSize(this@LauncherActivity, job.received)
            val total = if (job.total > 0) Formatter.formatFileSize(this@LauncherActivity, job.total) else "unknown size"
            val state = when (job.status) {
                "QUEUED" -> "Queued"
                "RUNNING" -> if (job.error != null) "Waiting to reconnect" else "Downloading"
                "PAUSED" -> "Paused"
                "FAILED" -> "Download failed"
                else -> job.status
            }
            (labels.getChildAt(1) as TextView).text = "$state · Downloaded $bytes of $total" +
                if (job.error != null) "\n${job.error}" else ""
            val progress = labels.getChildAt(2) as ProgressBar
            val active = job.status == "RUNNING" || job.status == "QUEUED"
            progress.isIndeterminate = active && (job.total <= 0 || job.status == "QUEUED" || job.error != null)
            val amount = if (job.total > 0) ((job.received.toDouble() / job.total) * 1000).toInt().coerceIn(0, 1000) else 0
            if (android.os.Build.VERSION.SDK_INT >= 24) progress.setProgress(amount, active)
            else progress.progress = amount
            progress.contentDescription = "${job.name}: $state, downloaded $bytes of $total"
            (row.getChildAt(2) as TextView).apply {
                contentDescription = "${job.name} download actions"
                setOnClickListener { downloadActions(job) }
            }
            return row
        }
    }

    private fun downloadActions(job: IpaDownloads.Job) {
        val active = job.status == "RUNNING" || job.status == "QUEUED"
        val action = if (active) "Pause download" else if (job.status == "FAILED") "Retry download" else "Resume download"
        AlertDialog.Builder(this).setTitle(job.name).setMessage(job.error ?: "Downloaded " +
            Formatter.formatFileSize(this, job.received))
            .setPositiveButton(action) { _, _ ->
                if (active) IpaDownloads.cancel(this, job.id) else IpaDownloads.retry(this, job.id)
            }.setNegativeButton("Close", null).show()
    }

    private fun bindDownloadIcon(image: ImageView, url: String?, name: String) {
        image.tag = url
        image.contentDescription = "$name icon"
        image.setImageBitmap(url?.let { iconCache.get(it) })
        image.visibility = if (url != null && iconCache.get(url) != null) View.VISIBLE else View.INVISIBLE
        if (url == null || iconCache.get(url) != null || failedIcons.contains(url) ||
            pendingIcons.containsKey(url) || pendingIcons.size >= 32) return
        val transfer = RepoSources.Transfer()
        pendingIcons[url] = transfer
        iconWorker.execute {
            val decoded = try {
                val bytes = RepoSources.fetchIcon(url, transfer)
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                require(bounds.outWidth > 0 && bounds.outHeight > 0 &&
                    bounds.outWidth.toLong() * bounds.outHeight <= 16_000_000)
                val options = BitmapFactory.Options().apply { inSampleSize = 1 }
                while (bounds.outWidth / options.inSampleSize > 256 || bounds.outHeight / options.inSampleSize > 256)
                    options.inSampleSize *= 2
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            } catch (_: Exception) { null } catch (_: OutOfMemoryError) { null }
            runOnUiThread {
                pendingIcons.remove(url)
                if (!isFinishing && !isDestroyed && !transfer.cancelled.get()) {
                    if (decoded != null) iconCache.put(url, decoded) else failedIcons.add(url)
                    adapter.notifyDataSetChanged()
                }
            }
        }
    }

    companion object {
        private const val REQUEST_PICK_IPA = 1
        private const val PREF_COMPAT = "compat_mode"

        private val BACKGROUND = Color.parseColor("#0E1014")
        private val CARD = Color.parseColor("#1A1D24")
        private val ACCENT = Color.parseColor("#7C5CFF")
        private val TEXT_DIM = Color.parseColor("#8B909C")

        private val PALETTE = arrayOf(
            intArrayOf(Color.parseColor("#7C5CFF"), Color.parseColor("#C25CFF")),
            intArrayOf(Color.parseColor("#FF6B6B"), Color.parseColor("#FFA86B")),
            intArrayOf(Color.parseColor("#1FB6A6"), Color.parseColor("#3EA0FF")),
            intArrayOf(Color.parseColor("#F5B83D"), Color.parseColor("#F06A3D")),
            intArrayOf(Color.parseColor("#4CC76A"), Color.parseColor("#17A2A2")),
            intArrayOf(Color.parseColor("#3D7BFF"), Color.parseColor("#5C3DFF")),
        )
    }
}
