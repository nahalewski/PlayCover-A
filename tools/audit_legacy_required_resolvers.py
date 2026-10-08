"""Inspect actual IPA import references, including weak providers present in cache."""
import json,pathlib,struct
root=pathlib.Path(__file__).resolve().parents[1]
reports=json.loads((root/'ios-runtime/legacy-cpp-consumer-audit.json').read_text())
cache=root/'ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/dyld_shared_cache_arm64'
def leb(b,p):
 v=0
 for i in range(10):
  x=b[p];p+=1;v|=(x&127)<<(7*i)
  if x<128:return v,p
 raise ValueError('LEB overflow')
with cache.open('rb') as f:
 h=f.read(256);mo,mc,io,ic=struct.unpack_from('<4I',h,16)
 f.seek(mo);maps=[struct.unpack('<QQQII',f.read(32)) for _ in range(mc)]
 def read(a,n):
  for base,size,off,_,_ in maps:
   if base<=a and a+n<=base+size:f.seek(off+a-base);return f.read(n)
  raise ValueError('VM range')
 paths={}
 for i in range(ic):
  f.seek(io+32*i);a,_,_,p,_=struct.unpack('<QQQII',f.read(32));f.seek(p)
  paths[f.read(4096).split(b'\0')[0].decode()]=a
 memo={}
 def meta(path):
  if path in memo:return memo[path]
  a=paths[path];h=read(a,32);cmds=read(a+32,struct.unpack_from('<I',h,20)[0]);p=0;deps=[];reexports=[];trie=b''
  for _ in range(struct.unpack_from('<I',h,16)[0]):
   c,s=struct.unpack_from('<II',cmds,p);b=cmds[p:p+s]
   if c in (0xc,0x80000018,0x8000001f,0x80000023):
    no=struct.unpack_from('<I',b,8)[0];deps.append(b[no:].split(b'\0')[0].decode())
    if c==0x8000001f:reexports.append(len(deps))
   if c in (0x22,0x80000022):
    off,size=struct.unpack_from('<II',b,40);f.seek(off);trie=f.read(size)
   if c==0x80000033:
    off,size=struct.unpack_from('<II',b,8);f.seek(off);trie=f.read(size)
   p+=s
  entries={};stack=[(0,'')]
  while stack and trie:
   pos,name=stack.pop();size,p=leb(trie,pos);end=p+size
   if size:
    flags,q=leb(trie,p);val,q=leb(trie,q)
    extra=(trie[q:end].split(b'\0')[0].decode() if flags&8 else leb(trie,q)[0] if flags&16 else None)
    entries[name]=(flags,val,extra)
   count=trie[end];p=end+1
   for _ in range(count):
    stop=trie.index(0,p);edge=trie[p:stop].decode();child,p=leb(trie,stop+1);stack.append((child,name+edge))
  memo[path]=(a,deps,reexports,entries);return memo[path]
 def resolve(path,name,seen=frozenset()):
  if path not in paths or (path,name) in seen:return None
  a,deps,reexp,entries=meta(path);seen=seen|{(path,name)};e=entries.get(name)
  if e:
   flags,value,extra=e
   if flags&8:return resolve(deps[value-1],extra or name,seen)
   if flags&16:return path,a+value,a+extra
   return None
  for ordinal in reexp:
   found=resolve(deps[ordinal-1],name,seen)
   if found:return found
 output={}
 for report in reports:
  for image in report['images']:
   deps=image['dependencies']
   for kind in ('bind','lazy'):
    for ordinal,name,weak,typ in image['streams'].get(kind,{}).get('references',[]):
     if not 0<ordinal<=len(deps):continue
     found=resolve(deps[ordinal-1]['name'],name)
     if found:
      provider,stub,resolver=found;key=(stub,resolver)
      entry=output.setdefault(key,{'provider':provider,'names':[],'consumers':[],'import_evidence':[],'stub':hex(stub),'resolver':hex(resolver),'bytes':read(resolver,44).hex()})
      if name not in entry['names']:entry['names'].append(name)
      consumer=pathlib.Path(report['ipa']).name+':'+image['path']
      if consumer not in entry['consumers']:entry['consumers'].append(consumer)
      proof={'consumer':consumer,'stream':kind,'library_ordinal':ordinal,'requested_provider':deps[ordinal-1]['name'],'symbol':name,'weak_import':weak}
      if proof not in entry['import_evidence']:entry['import_evidence'].append(proof)
 out=list(output.values());(root/'ios-runtime/legacy-required-resolvers.json').write_text(json.dumps(out,indent=2)+'\n')
 for e in out:print(e['names'],e['stub'],e['resolver'],e['bytes'])
 print('actual required resolver pairs',len(out))
