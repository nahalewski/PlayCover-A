"""Static Mach-O requirements for the user's Sonic archives; no guest execution."""
import hashlib,json,plistlib,struct,zipfile
from pathlib import Path
reports=[]
for ipa in Path('pixel-fold-tests').glob('Sonic*-icons.ipa'):
    report={'source':str(ipa),'sha256':hashlib.sha256(ipa.read_bytes()).hexdigest(),'slices':[]}
    with zipfile.ZipFile(ipa) as z:
        infopath=next(n for n in z.namelist() if n.count('/')==2 and n.endswith('/Info.plist'))
        info=plistlib.loads(z.read(infopath)); report['bundle']={k:info.get(k) for k in ['CFBundleIdentifier','CFBundleDisplayName','CFBundleVersion','MinimumOSVersion','NSMainNibFile','UIDeviceFamily']}
        raw=z.read(infopath.removesuffix('Info.plist')+info['CFBundleExecutable'])
        slices=[]
        if raw[:4]==bytes.fromhex('cafebabe'):
            for i in range(struct.unpack_from('>I',raw,4)[0]):
                cpu,sub,off,size,_=struct.unpack_from('>5I',raw,8+20*i); slices.append((cpu,sub,raw[off:off+size]))
        else: slices.append((*struct.unpack_from('<II',raw,4),raw))
        for cpu,sub,data in slices:
            r={'cpu':cpu,'subtype':sub,'size':len(data),'libraries':[],'cryptid':None,'sections':[],'imports':[]}; report['slices'].append(r)
            sections={}; pos=28 if data[:4]==bytes.fromhex('cefaedfe') else 32
            for _ in range(struct.unpack_from('<I',data,16)[0]):
                cmd,size=struct.unpack_from('<II',data,pos)
                if cmd in (12,0x80000018,0x8000001f):
                    off=struct.unpack_from('<I',data,pos+8)[0];r['libraries'].append({'path':data[pos+off:pos+size].split(b'\0')[0].decode(),'weak':cmd==0x80000018})
                elif cmd==0x21:r['cryptid']=struct.unpack_from('<I',data,pos+16)[0]
                elif cmd==1:
                    for i in range(struct.unpack_from('<I',data,pos+48)[0]):
                        off=pos+56+68*i;name=data[off:off+16].split(b'\0')[0].decode();addr,length,fileoff=struct.unpack_from('<III',data,off+32);sections[name]=(length,fileoff);r['sections'].append(name)
                elif cmd==2:
                    symoff,n,stroff,strlen=struct.unpack_from('<4I',data,pos+8);strings=data[stroff:stroff+strlen]
                    for i in range(n):
                        idx,typ,sect,desc,value=struct.unpack_from('<IBBHI',data,symoff+12*i)
                        if typ&14==0 and value==0 and idx<len(strings):r['imports'].append(strings[idx:strings.find(b'\0',idx)].decode(errors='replace'))
                pos+=size
            for section,key in [('__objc_methname','selectors'),('__objc_classname','classes')]:
                if section in sections:
                    length,off=sections[section];r[key]=sorted(set(x.decode(errors='replace') for x in data[off:off+length].split(b'\0') if x))
        report['nibs']={}
        for name in z.namelist():
            if name.endswith('.nib'):
                try:
                    objects=plistlib.loads(z.read(name))['$objects']; report['nibs'][name]={'classes':sorted(set(o['$classname'] for o in objects if isinstance(o,dict) and '$classname' in o)),'keys':sorted(set(k for o in objects if isinstance(o,dict) for k in o if not k.startswith('$')))}
                except Exception as e:report['nibs'][name]={'error':str(e)}
    reports.append(report)
Path('sonic-runtime-requirements.json').write_text(json.dumps(reports,indent=2)+'\n',encoding='utf-8')
for r in reports:
    print(r['source'],r['bundle'])
    for s in r['slices']:
        print('CPU',s['cpu'],'subtype',s['subtype'],'cryptid',s['cryptid'],'imports',len(s['imports']))
        print('libraries',s['libraries']);print('NIB count',len(r['nibs']))
