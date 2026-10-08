"""Read-only SDK previous-install-name and actual cache export audit."""
import pathlib,json,struct,zipfile,re
root=pathlib.Path(__file__).resolve().parents[1]
source=(root/'tools/inspect_legacy_ipa_runtime.py').read_text().split('ipa, cache_root, output =')[0]
source=source.replace("return {\n        'opcode_counts'","return {\n        'references': sorted(references),\n        'opcode_counts'")
ns={};exec(compile(source,'scanner','exec'),ns)
plan=json.loads((root/'ios-runtime/cache-loader-plan.json').read_text())
cache=root/'ios-runtime/cache/System/Library/Caches/com.apple.dyld'
def read(address,size):
    region=next(r for r in plan['intervals'] if r['address']<=address and address+size<=r['end_address'])
    with (cache/region['file']).open('rb') as f:
        f.seek(region['file_offset']+address-region['address']);data=f.read(size)
    if len(data)!=size:raise ValueError('short original cache read')
    return data
provider=next(i for i in plan['images'] if i['path']=='/System/Library/Frameworks/CloudKit.framework/CloudKit')
h=read(provider['address'],32);commands=read(provider['address']+32,struct.unpack_from('<I',h,20)[0]);off=0;segments=[];executable=[]
for _ in range(struct.unpack_from('<I',h,16)[0]):
    cmd,size=struct.unpack_from('<II',commands,off)
    if cmd==25:
        segment=struct.unpack_from('<QQQQ',commands,off+24);segments.append(segment)
        if struct.unpack_from('<I',commands,off+60)[0]&4:executable.append((segment[0],segment[0]+segment[1]))
    if cmd in (0x22,0x80000022):eo,es=struct.unpack_from('<II',commands,off+40)
    if cmd==0x80000033:eo,es=struct.unpack_from('<II',commands,off+8)
    off+=size
segment=next(s for s in segments if s[2]<=eo and eo+es<=s[2]+s[3]);trie=read(segment[0]+eo-segment[2],es)
tbd=(root/'ios-runtime/sdk/theos-reference/iPhoneOS16.5.sdk/System/Library/Frameworks/CloudKit.framework/CloudKit.tbd').read_text()
old='/usr/lib/swift/libswiftCloudKit.dylib';images=[]
with zipfile.ZipFile(pathlib.Path(r'C:/Users/Ben/Desktop/ipa/Dead_Cells_plus_v3.2.2.ipa')) as archive:
    for item in archive.infolist():
        if item.file_size<32:continue
        with archive.open(item) as f:magic=f.read(4)
        if magic not in (b'\xcf\xfa\xed\xfe',b'\xca\xfe\xba\xbe'):continue
        try:image=ns['inspect_image'](item.filename,archive.read(item),root/'ios-runtime/extractedlibs')
        except Exception:continue
        dep=next((d for d in image['dependencies'] if d['name']==old),None)
        if not dep:continue
        required=sorted({r[1] for kind in ('bind','lazy') for r in image['streams'].get(kind,{}).get('references',[]) if r[0]==dep['ordinal'] and not r[2]})
        exports=ns['selected_exports'](trie,required)
        for entry in exports.values():
            if entry.get('strong_exported_definition') and not entry.get('reexport'):
                target=provider['address']+int(entry['value'],0)
                entry['actual_target']=hex(target)
                entry['target_inside_provider_executable_segment']=any(start<=target and target+4<=end for start,end in executable)
                entry['original_instruction_bytes']=read(target,4).hex()
        previous={symbol:re.findall(re.escape('$ld$previous$'+old)+'[^\n]*'+re.escape(symbol)+r'[^\n]*',tbd) for symbol in required}
        images.append({'path':image['path'],'dependency':dep,'required_strong':required,'actual_cache_exports':exports,'sdk_previous_directives':previous})
report={'old_path':old,'actual_provider':provider,'original_cache_uuid':plan.get('uuid'),'scope':'metadata; no alias installed or execution claimed','images':images}
out=root/'ios-runtime/cloudkit-migration-audit.json';out.write_text(json.dumps(report,indent=2)+'\n')
for image in images:
    print(image['path'],'strongimports',len(image['required_strong']),'missingexports',[s for s,e in image['actual_cache_exports'].items() if e.get('absent')],'missingprevious',[s for s,d in image['sdk_previous_directives'].items() if not d])
print(out)
