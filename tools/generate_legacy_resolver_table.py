"""Generate finite original-cache selectors from the actual-import audit."""
import json,pathlib,struct,re
root=pathlib.Path(__file__).resolve().parents[1]
entries=json.loads((root/'ios-runtime/legacy-required-resolvers.json').read_text())
def adrp(i,pc,reg):
 assert i&0x9f00001f==0x90000000|reg
 imm=((i>>5)&0x7ffff)*4+((i>>29)&3)
 if imm&(1<<20):imm-=1<<21
 return (pc&~4095)+(imm<<12)
rows=[]
for entry in entries:
 b=bytes.fromhex(entry['bytes']);w=struct.unpack('<11I',b);pc=int(entry['resolver'],16)
 assert w[:3]==(0xd2980468,0xf2bfffe8,0xf2c001e8)
 assert w[5]==0x39400108 and w[8:]==(0x721f011f,0x9a890140,0xd65f03c0)
 assert w[4]&0xffc003ff==0x91000129 and w[7]&0xffc003ff==0x9100014a
 alternative=adrp(w[3],pc+12,9)+((w[4]>>10)&4095)
 target=adrp(w[6],pc+24,10)+((w[7]>>10)&4095)
 entry['baseline_target']=hex(target);entry['lse_target']=hex(alternative)
 rows.append('    // '+', '.join(entry['names'])+'; actual IPA imports (present weak included).\n    ('+entry['stub']+', '+entry['resolver']+', ['+', '.join(hex(x) for x in b)+'], '+hex(target)+'),')
table='// Finite set from tools/audit_legacy_required_resolvers.py; original 15G77 UUID only.\nconst IOS11_REQUIRED_SELECTORS: &[(u64, u64, [u8; 44], u64)] = &[\n'+'\n'.join(rows)+'\n];\n'
p=root/'touchHLE-src/src/a64_cache_resolver.rs';s=p.read_text()
if 'const IOS11_REQUIRED_SELECTORS:' in s:
 s,count=re.subn(r'const IOS11_REQUIRED_SELECTORS:.*?\n\];',table.split('\n',1)[1].rstrip(),s,count=1,flags=re.S)
 assert count==1
 s=re.sub(r'assert_eq!\(IOS11_REQUIRED_SELECTORS.len\(\), \d+\);',f'assert_eq!(IOS11_REQUIRED_SELECTORS.len(), {len(rows)});',s)
 p.write_text(s)
 (root/'ios-runtime/legacy-required-resolvers.json').write_text(json.dumps(entries,indent=2)+'\n')
 print('Updated finite table:',len(rows),'structurally verified actual import selectors.')
 raise SystemExit(0)
anchor='const IOS11_UUID:';s=s.replace(anchor,table+'\n'+anchor,1)
old='''                let entry = (self.uuid == IOS16_UUID)
                    .then(|| {'''
new='''                if self.uuid == IOS11_UUID {
                    let entry = IOS11_REQUIRED_SELECTORS.iter()
                        .find(|entry| entry.0 == stub && entry.1 == resolver)
                        .ok_or_else(|| format!("unaudited cache resolver {resolver:#x} (stub {stub:#x})"))?;
                    (&entry.2[..], entry.3)
                } else {
                let entry = (self.uuid == IOS16_UUID)
                    .then(|| {'''
assert old in s;s=s.replace(old,new,1)
old='''                (&entry.2, entry.3)
            }
        };''';new='''                (&entry.2[..], entry.3)
                }
            }
        };''';assert old in s;s=s.replace(old,new,1)
start=s.index('        for (stub, resolver, target, bytes) in [',s.index('fn ios11_barrier_selector'))
end=s.index('        let mut cpu',start)
s=s[:start]+'''        assert_eq!(IOS11_REQUIRED_SELECTORS.len(), 32);
        for &(stub, resolver, bytes, target) in IOS11_REQUIRED_SELECTORS {
'''+s[end:]
p.write_text(s)
(root/'ios-runtime/legacy-required-resolvers.json').write_text(json.dumps(entries,indent=2)+'\n')
print('validated and generated',len(rows),'actual import selectors')
