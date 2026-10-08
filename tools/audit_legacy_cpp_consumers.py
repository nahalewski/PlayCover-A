"""Read-only exact legacy-cache export/consumer-import ABI audit."""
import pathlib,struct,json,zipfile
root=pathlib.Path(__file__).resolve().parents[1]
source=(root/'tools/inspect_legacy_ipa_runtime.py').read_text().split('ipa, cache_root, output =')[0]
source=source.replace("return {\n        'opcode_counts'","return {\n        'references': sorted(references),\n        'opcode_counts'")
ns={};exec(compile(source,'inspect_legacy_ipa_runtime.py','exec'),ns)
cache=root/'ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64'
with cache.open('rb') as f:
    header=f.read(256);mo,mc,io,ic=struct.unpack_from('<4I',header,16)
    f.seek(mo);maps=[struct.unpack('<QQQII',f.read(32)) for _ in range(mc)]
    def read_vm(address,size):
        for base,length,offset,_,_ in maps:
            if base<=address and address+size<=base+length:
                f.seek(offset+address-base);return f.read(size)
        raise ValueError('original cache VM outside mappings')
    paths={}
    for i in range(ic):
        f.seek(io+i*32);address,_,_,path,_=struct.unpack('<QQQII',f.read(32))
        f.seek(path);paths[f.read(4096).split(b'\0')[0].decode()]=address
    address=paths['/usr/lib/libstdc++.6.dylib'];h=read_vm(address,32)
    cmds=read_vm(address+32,struct.unpack_from('<I',h,20)[0]);off=0
    for _ in range(struct.unpack_from('<I',h,16)[0]):
        cmd,size=struct.unpack_from('<II',cmds,off)
        if cmd in (0x22,0x80000022):
            exportoff,exportsize=struct.unpack_from('<II',cmds,off+40)
        off+=size
    f.seek(exportoff);trie=f.read(exportsize)
reports=[]
for ipa in [pathlib.Path(r'C:/Users/Ben/Desktop/ipa/Infinity-Blade-II-v1-3-5.ipa'),pathlib.Path(r'C:/Users/Ben/Desktop/ipa/Infinity-Blade-III-v1-4-4.ipa'),root/'PokemonQuest-device.ipa']:
    report={'ipa':str(ipa),'images':[]}
    with zipfile.ZipFile(ipa) as z:
        for entry in z.infolist():
            if entry.file_size<32:continue
            with z.open(entry) as stream:magic=stream.read(4)
            if magic not in (b'\xcf\xfa\xed\xfe',b'\xca\xfe\xba\xbe'):continue
            try:
                raw=z.read(entry);data,_=ns['thin_arm64'](raw)
                image=ns['inspect_image'](entry.filename,raw,root/'ios-runtime/extractedlibs')
            except Exception as error:
                report.setdefault('unparsed',[]).append({'path':entry.filename,'error':str(error)});continue
            image['missing_strong_oldcache_paths']=[d['name'] for d in image['dependencies'] if not d['weak'] and d['name'].startswith('/') and d['name'] not in paths]
            requirements=[]
            for dep in image['dependencies']:
                if 'libstdc++' in dep['name']:
                    requirements.extend(r[1] for kind in ('bind','lazy') for r in image['streams'].get(kind,{}).get('references',[]) if r[0]==dep['ordinal'] and not r[2])
            requirements=sorted(set(requirements));image['cpp_required']=requirements
            image['cpp_exports']=ns['selected_exports'](trie,requirements)
            off=32;versions=[]
            for _ in range(struct.unpack_from('<I',data,16)[0]):
                cmd,size=struct.unpack_from('<II',data,off)
                if cmd==0x25:versions.append({'minimum':hex(struct.unpack_from('<I',data,off+8)[0]),'sdk':hex(struct.unpack_from('<I',data,off+12)[0])})
                if cmd==0x32:versions.append({'platform':struct.unpack_from('<I',data,off+8)[0],'minimum':hex(struct.unpack_from('<I',data,off+12)[0]),'sdk':hex(struct.unpack_from('<I',data,off+16)[0])})
                off+=size
            image['version_commands']=versions
            report['images'].append(image)
    reports.append(report)
output=root/'ios-runtime/legacy-cpp-consumer-audit.json';output.write_text(json.dumps(reports,indent=2)+'\n')
for report in reports:
    print(pathlib.Path(report['ipa']).name,'images',len(report['images']))
    for image in report['images']:
        missing=[s for s,e in image['cpp_exports'].items() if e.get('absent')]
        print(image['path'],image['version_commands'],'C++',len(image['cpp_required']),'missingexports',missing,'missingoldpaths',image['missing_strong_oldcache_paths'])
print(output)
