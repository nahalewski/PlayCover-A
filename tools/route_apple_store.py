from pathlib import Path
p=Path('touchHLE-src/android/app/src/main/java/org/touchhle/android/LauncherActivity.kt')
s=p.read_text(encoding='utf-8')
start=s.index('    private fun showAppleStore() {')
end=s.index('    private fun showSettings() {',start)
s=s[:start]+'''    private fun showAppleStore() {
        repositoryTransfer?.cancel()
        startActivity(Intent(this, AppleStoreActivity::class.java))
    }

'''+s[end:]
needle='        content.addView(sectionLabel("DISPLAY"))'
s=s.replace(needle,'''        content.addView(sectionLabel("APP STORE"))
        content.addView(actionButton("Apple account and connection") {
            startActivity(Intent(this, AppleStoreActivity::class.java).putExtra("settings", true))
        })
'''+needle,1)
p.write_text(s,encoding='utf-8')
