from pathlib import Path
p = Path('touchHLE-src/android/app/src/main/java/org/touchhle/android/LauncherActivity.kt')
s = p.read_text(encoding='utf-8')
s = s.replace('"settings" -> showSettings()', '"settings" -> showSettings()\n                "store" -> showAppleStore() ', 1)
s = s.replace('"sources", "settings" -> showLibrary()', '"sources", "settings", "store" -> showLibrary()')
s = s.replace('"sources" to "Repositories", "settings" to "Settings"', '"sources" to "Repositories", "store" to "App Store", "settings" to "Settings"')
s = s.replace('"sources" -> showRepositories(); else -> showSettings()', '"sources" -> showRepositories(); "store" -> showAppleStore(); else -> showSettings()')
method = '''
    private fun showAppleStore() {
        repositoryTransfer?.cancel(); repositoryPage = "store"; repositorySourceURL = null
        val page = repositoryPageView("App Store") { showLibrary() }
        page.addView(sectionLabel("Search Apple's catalog. App Store IPAs may be encrypted and cannot yet run in PlayCover-A."))
        val query = EditText(this).apply { hint = "Search iPhone apps"; setTextColor(Color.WHITE); setHintTextColor(TEXT_DIM); isSingleLine = true }
        page.addView(query)
        val results = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val scroll = ScrollView(this).apply { addView(results) }
        page.addView(actionButton("Search") {
            val term = query.text.toString().trim()
            if (term.isNotEmpty()) {
                results.removeAllViews(); results.addView(sectionLabel("Searching…"))
                repositoryWorker.execute {
                    try {
                        val url = java.net.URL("https://itunes.apple.com/search?entity=software&limit=30&term=" + java.net.URLEncoder.encode(term, "UTF-8"))
                        val connection = url.openConnection().apply { connectTimeout = 15000; readTimeout = 15000 }
                        val data = connection.getInputStream().bufferedReader().use { JSONObject(it.readText()).getJSONArray("results") }
                        runOnUiThread {
                            if (repositoryPage == "store") {
                                results.removeAllViews()
                                if (data.length() == 0) results.addView(sectionLabel("No apps found"))
                                for (i in 0 until data.length()) {
                                    val app = data.getJSONObject(i)
                                    results.addView(actionButton(app.optString("trackName") + " · iOS " + app.optString("minimumOsVersion")) {
                                        val link = app.optString("trackViewUrl")
                                        if (link.startsWith("https://")) startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link)))
                                    })
                                }
                            }
                        }
                    } catch (e: Exception) {
                        runOnUiThread { if (repositoryPage == "store") { results.removeAllViews(); results.addView(sectionLabel("Search failed: " + (e.message ?: "network error"))) } }
                    }
                }
            }
        })
        page.addView(actionButton("Apple account downloads") {
            AlertDialog.Builder(this).setTitle("Apple account downloads")
                .setMessage("Authenticated downloads require the local ipatool companion. Native Apple-account login is not connected yet. No password is collected by this page.")
                .setPositiveButton("OK", null).show()
        })
        page.addView(scroll, LinearLayout.LayoutParams(-1, 0, 1f))
        displayPage(page, "store")
    }

'''
s = s.replace('    private fun showSettings() {', method + '    private fun showSettings() {')
p.write_text(s, encoding='utf-8')
