"""Read-only original iOS11 libSystem bootstrap inventory, separate from iOS16."""
import pathlib,struct,json,importlib.util
root=pathlib.Path(__file__).resolve().parents[1]
cache=root/'ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64'
with cache.open('rb') as f:
 h=f.read(256)
 assert h[88:104].hex()=='7336d75f301433e7843fe1f3522fc52f'
 mo,mc,io,ic=struct.unpack_from('<4I',h,16);so,ss=struct.unpack_from('<QQ',h,56)
 f.seek(so);s=f.read(48)
 assert struct.unpack_from('<I',s)[0]==2
 mask,add=struct.unpack_from('<QQ',s,24)
 f.seek(mo);maps=[struct.unpack('<QQQII',f.read(32)) for _ in range(mc)]
 intervals=[{'file':cache.name,'address':a,'end_address':a+n,'file_offset':o,'max_prot':mx,'init_prot':ip,'slide_info':{'delta_mask':hex(mask),'value_add':add} if ip&2 else None} for a,n,o,mx,ip in maps]
 images=[]
 for i in range(ic):
  f.seek(io+i*32);a,_,_,p,_=struct.unpack('<QQQII',f.read(32));f.seek(p);path=f.read(4096).split(b'\0')[0].decode()
  images.append({'path':path,'address':a,'address_hex':hex(a)})
plan=root/'ios-runtime/legacy-cache-loader-plan.json'
plan.write_text(json.dumps({'cache_root':str(cache.parent),'cache_uuid':h[88:104].hex(),'intervals':intervals,'images':images},indent=2)+'\n')
spec=importlib.util.spec_from_file_location('audit',root/'tools/audit_cached_initializers.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
result=m.audit(plan,['/usr/lib/libSystem.B.dylib','/usr/lib/system/libdyld.dylib','/usr/lib/system/libsystem_pthread.dylib'])
out=root/'ios-runtime/legacy-libsystem-initializer-audit.json';out.write_text(json.dumps(result,indent=2)+'\n')
print([(i['path'],[c['target'] for c in i['initializers']]) for i in result['images']]);print(out)
