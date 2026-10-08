from pathlib import Path
import re
p=Path('touchHLE-src/android/app/src/main/java/org/touchhle/android/AppleStoreActivity.kt')
s=p.read_text(encoding='utf-8')
s=re.sub(r'"to\b','" to',s)
s=re.sub(r'"else\b','" else ',s)
s=s.replace('private val background=','private val surfaceColor=')
s=s.replace('statusBarColor=background','statusBarColor=surfaceColor').replace('navigationBarColor=background','navigationBarColor=surfaceColor')
s=s.replace('setBackgroundColor(background)','setBackgroundColor(surfaceColor)').replace('cacheColorHint=background','cacheColorHint=surfaceColor')
p.write_text(s,encoding='utf-8')
