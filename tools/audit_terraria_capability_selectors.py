#!/usr/bin/env python3
"""Audit only actual Unity/PlayFab undefined imports against known resolver exports.

Metadata-only: no guest execution. Full original-cache disassembly is retained
for review, and exact resolver bytes are accepted only for the established
40-byte capability selector form (PC-relative targets may differ).
"""
import json,re,struct,subprocess,zipfile
from pathlib import Path
root=Path(__file__).resolve().parent.parent
ipa=Path('C:/Users/Ben/Desktop/ipa/Terraria_4.5.0.ipa')
providers={}
for filename in ['platform-resolver-exports.tsv','pthread-resolver-exports.tsv','dispatch-resolver-exports.tsv']:
    for line in (root/'pixel-fold-tests'/filename).read_text().splitlines():
        name,stub,resolver=line.split('\t');providers[name]=(stub,resolver)
def undefined(data):
    magic=int.from_bytes(data[:4],'big')
    if magic in (0xcafebabe,0xcafebabf):
        count=struct.unpack_from('>I',data,4)[0]
        if count>32:raise ValueError('fat architecture bound exceeded')
        for i in range(count):
            offset=8+i*(32 if magic==0xcafebabf else 20)
            cpu=struct.unpack_from('>I',data,offset)[0]
            if cpu==0x100000c:
                start,size=struct.unpack_from('>QQ' if magic==0xcafebabf else '>II',data,offset+8)
                if start+size>len(data):raise ValueError('fat slice outside file')
                return undefined(data[start:start+size])
        raise ValueError('no ARM64 slice')
    if data[:4]!=bytes.fromhex('cffaedfe'):raise ValueError('not ARM64 thin Mach-O')
    cpu,_,_,commands,command_bytes=struct.unpack_from('<5I',data,4)
    if cpu!=0x100000c or commands>4096 or 32+command_bytes>len(data):raise ValueError('invalid ARM64 load commands')
    pos=32;symtab=None
    for _ in range(commands):
        cmd,size=struct.unpack_from('<II',data,pos)
        if size<8 or pos+size>32+command_bytes:raise ValueError('invalid load command range')
        if cmd==2:symtab=struct.unpack_from('<4I',data,pos+8)
        pos+=size
    if not symtab:raise ValueError('no symtab')
    off,count,strings,string_size=symtab
    if count>1000000 or off+count*16>len(data) or strings+string_size>len(data):raise ValueError('symtab bounds')
    names=set()
    for i in range(count):
        string,kind,section,desc,value=struct.unpack_from('<IBBHQ',data,off+i*16)
        if kind&0x0e or not(kind&1):continue
        if string>=string_size:raise ValueError('invalid string index')
        end=data.find(b'\0',strings+string,strings+string_size)
        if end<0:raise ValueError('unterminated name')
        names.add(data[strings+string:end].decode())
    return names
records=[]
with zipfile.ZipFile(ipa) as archive:
    for framework in ['UnityFramework','PlayFabParty']:
        names=undefined(archive.read(f'Payload/Terraria.app/Frameworks/{framework}.framework/{framework}'))
        for name in sorted(names & providers.keys()):
            stub,resolver=providers[name]
            command=[str(root/'ios-runtime/tools/ipsw.exe'),'dyld','disass',str(root/'ios-runtime/cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64'),'--vaddr',resolver,'--count','10','--quiet']
            disassembly=subprocess.check_output(command,text=True)
            rows=re.findall(r'^0x[0-9a-f]+:\s+((?:[0-9a-f]{2}\s+){3}[0-9a-f]{2})\s+(.+)$',disassembly,re.M)
            if len(rows)!=10:raise ValueError('incomplete selector disassembly')
            words=[int.from_bytes(bytes.fromhex(raw),'little') for raw,_ in rows]
            fixed={0:0xb2704fe8,1:0xf2980468,4:0x39400108,7:0x721f011f,8:0x9a890140,9:0xd65f03c0}
            safe=all(words[i]==word for i,word in fixed.items()) and words[2]&0x9f00001f==0x90000009 and words[5]&0x9f00001f==0x9000000a and words[3]&0xffc003ff==0x91000129 and words[6]&0xffc003ff==0x9100014a
            def target(index):
                pc=int(resolver,16)+index*4;word=words[index]
                imm=((word>>5)&0x7ffff)<<2|((word>>29)&3)
                if imm&(1<<20):imm-=1<<21
                return (pc&~4095)+(imm<<12)+((words[index+1]>>10)&0xfff)
            records.append({'framework':framework,'name':name,'stub':stub,'resolver':resolver,'exact_bytes':''.join(raw.replace(' ','') for raw,_ in rows),'bounded_capability_form':safe,'non_lse_target':hex(target(5)) if safe else None,'lse_target':hex(target(2)) if safe else None,'disassembly':disassembly})
report={'scope':'Undefined-symbol metadata for actual UnityFramework/PlayFabParty, original-cache disassembly only; no guest execution','selectors':records}
path=root/'pixel-fold-tests/terraria-capability-selector-audit.json';path.write_text(json.dumps(report,indent=2)+'\n')
for record in records:print(record['framework'],record['name'],record['resolver'],record['bounded_capability_form'],record['non_lse_target'])
