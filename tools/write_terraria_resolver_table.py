#!/usr/bin/env python3
"""Generate a finite, reviewable whitelist from the saved targeted disassembly.

This tool does not enable runtime pattern matching. Every emitted entry retains
an exact original-cache stub, resolver, 40-byte body and non-LSE target.
"""
import json
from pathlib import Path
root=Path(__file__).resolve().parent.parent
records=json.loads((root/'pixel-fold-tests/terraria-capability-selector-audit.json').read_text())['selectors']
entries={}
for r in records:
    if not r['bounded_capability_form']:raise ValueError('unaudited form')
    key=(r['stub'],r['resolver'])
    if key in entries and entries[key]!=r['exact_bytes']:raise ValueError('inconsistent duplicate body')
    entries[key]=r['exact_bytes']
unique={(r['stub'],r['resolver']):r for r in records}
lines=['// Finite original iOS16 cache selectors actually imported by Terraria Unity/PlayFab.',
       '// Evidence: pixel-fold-tests/terraria-capability-selector-audit.json.',
       'const TERRARIA_CAPABILITY_SELECTORS: &[(u64, u64, [u8; 40], u64)] = &[']
for r in sorted(unique.values(),key=lambda r:int(r['resolver'],16)):
    raw=bytes.fromhex(r['exact_bytes'])
    if len(raw)!=40:raise ValueError('selector size')
    lines.append('    // '+r['name'])
    lines.append('    ('+r['stub']+', '+r['resolver']+', [')
    for start in range(0,40,10):lines.append('        '+', '.join(f'0x{x:02x}' for x in raw[start:start+10])+',')
    lines.append('    ], '+r['non_lse_target']+'),')
lines.append('];\n')
path=root/'touchHLE-src/src/a64_cache_resolver.rs';source=path.read_text()
marker='/// A deliberately small production resolver service.'
if 'const TERRARIA_CAPABILITY_SELECTORS:' in source:
    start=source.index('// Finite original iOS16 cache selectors')
    end=source.index(marker,start)
    source=source[:start]+'\n'.join(lines)+'\n'+source[end:]
else:
    source=source.replace(marker,'\n'.join(lines)+'\n'+marker,1)
path.write_text(source)
print('Generated',len(unique),'exact tuple entries')
