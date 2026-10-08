"""Read-only Mach-O pthread_create stub/direct branch audit, not execution."""
import json, struct, sys, zipfile
from pathlib import Path

def inspect(data):
    if len(data)<32 or struct.unpack_from('<II',data) != (0xfeedfacf,0x100000c):
        return None
    sections=[]; symbols=[]; indirect=[]; position=32
    for _ in range(struct.unpack_from('<I',data,16)[0]):
        cmd,size=struct.unpack_from('<II',data,position)
        if size<8 or position+size>len(data): raise ValueError('command bounds')
        if cmd==0x19:
            count=struct.unpack_from('<I',data,position+64)[0]
            if 72+80*count>size: raise ValueError('section bounds')
            for i in range(count):
                p=position+72+80*i
                name=data[p:p+16].split(b'\0')[0].decode()
                address,length,offset=struct.unpack_from('<QQI',data,p+32)
                flags,index,stride=struct.unpack_from('<III',data,p+64)
                sections.append((name,address,length,offset,flags,index,stride))
        elif cmd==2:
            table,count,strings,length=struct.unpack_from('<IIII',data,position+8)
            if table+16*count>len(data) or strings+length>len(data): raise ValueError('symbol bounds')
            for i in range(count):
                index=struct.unpack_from('<I',data,table+16*i)[0]
                if index>=length: raise ValueError('string bounds')
                end=data.find(b'\0',strings+index,strings+length)
                if end<0: raise ValueError('unterminated symbol')
                symbols.append(data[strings+index:end].decode(errors='replace'))
        elif cmd==0xb:
            offset,count=struct.unpack_from('<II',data,position+56)
            if offset+4*count>len(data): raise ValueError('indirect bounds')
            indirect=list(struct.unpack_from('<'+'I'*count,data,offset))
        position+=size
    targets={}
    for name,address,length,offset,flags,index,stride in sections:
        if flags&255!=8 or not stride: continue
        for i in range(length//stride):
            if index+i>=len(indirect): raise ValueError('stub index bounds')
            symbol=indirect[index+i]
            if symbol&0xc0000000: continue
            if symbol>=len(symbols): raise ValueError('stub symbol bounds')
            if symbols[symbol]=='_pthread_create': targets[address+i*stride]=[]
    for name,address,length,offset,*_ in sections:
        if name!='__text': continue
        if offset+length>len(data): raise ValueError('text bounds')
        for i in range(0,length-3,4):
            word=struct.unpack_from('<I',data,offset+i)[0]
            if word&0xfc000000 not in (0x94000000,0x14000000): continue
            displacement=word&0x3ffffff
            if displacement&0x2000000: displacement-=0x4000000
            target=address+i+displacement*4
            if target in targets:
                targets[target].append(dict(pc=hex(address+i),instruction='BL' if word&0xfc000000==0x94000000 else 'B',surrounding_bytes=data[offset+max(0,i-16):offset+min(length,i+20)].hex()))
    return dict(stubs=[dict(address=hex(target),direct_callers=calls) for target,calls in sorted(targets.items())])

result={}
with zipfile.ZipFile(sys.argv[1]) as archive:
    for name in ['Payload/Terraria.app/Frameworks/UnityFramework.framework/UnityFramework','Payload/Terraria.app/Frameworks/PlayFabParty.framework/PlayFabParty']:
        result[name]=inspect(archive.read(name))
report=dict(source=sys.argv[1],scope='Exact pthread_create symbol stubs and direct A64 B/BL call-site addresses; no indirect calls, register-value proof or execution',binaries=result)
Path(sys.argv[2]).write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps({name:dict(stubs=len(record['stubs']),direct_callers=sum(len(s['direct_callers']) for s in record['stubs'])) for name,record in result.items()}))
