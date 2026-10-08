"""Combine public source metadata into a conservative games-only AltStore feed.

No IPAs are fetched. Uncertain apps and emulators are omitted. Source failures
are recorded; duplicate version downloads prefer newest published date, then
the earliest listed source when dates do not establish precedence.
"""
import concurrent.futures,datetime,hashlib,json,re,time,urllib.parse,urllib.request
from pathlib import Path
GIST='https://api.github.com/gists/b40620d8d4a98ab17642858dce4cb2ec'
LIMIT=32*1024*1024
class Redirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,req,fp,code,msg,headers,url):
        if urllib.parse.urlsplit(url).scheme!='https':raise ValueError('HTTPS downgrade rejected')
        return super().redirect_request(req,fp,code,msg,headers,url)
def https(url):
    p=urllib.parse.urlsplit(url)
    return p.scheme=='https' and bool(p.hostname) and not p.username and not p.password and not p.fragment
def fetch(url):
    if not https(url):raise ValueError('Invalid HTTPS URL')
    deadline=time.monotonic()+35
    with urllib.request.build_opener(Redirect()).open(urllib.request.Request(url,headers={'User-Agent':'Mozilla/5.0','Accept':'application/json'}),timeout=12) as response:
        chunks=[];size=0
        while True:
            if time.monotonic()>deadline:raise TimeoutError('Source deadline exceeded')
            block=response.read(65536)
            if not block:break
            size+=len(block)
            if size>LIMIT:raise ValueError('Source exceeds32MiB')
            chunks.append(block)
        return b''.join(chunks),response.geturl()
KNOWN=re.compile(r'\b(minecraft|terraria|stardew|sonic|angry birds|pocket god|dead cells|geometry dash|subway surfers|temple run|deltarune|balatro|fortnite|little nightmares|hollow ?knight|true skate|pokemon|pokémon|pokemmo|pou|plants vs zombies|fruit ninja|jetpack joyride|grand theft auto|gta|roblox|among us|monument valley|cut the rope|limbo|inside|brawl stars|clash of clans|clash royale|call of duty|pubg|genshin impact|undertale|cuphead|binding of isaac|super mario|five nights at freddy|fnaf)\b')
EMULATOR=re.compile(r'\b(emulator|emulation|retroarch|ppsspp|dolphinios|provenance|inds|desmume|gba4ios|delta|ignited|folium|flycast|utm|ish|mame4ios|emuthreeds|firebird|pomelo|stiknes|play!)\b')
def classify(app):
    name=str(app.get('name',app.get('appname',''))).lower().replace('_',' ')
    desc=str(app.get('localizedDescription',app.get('description','')))[:8192].lower()
    identifier=str(app.get('bundleIdentifier',app.get('bundleid',''))).lower()
    if re.search(r'\b(?:news|reviews|manual|guide)\b',name):return 'other'
    # Some mixed feeds put emulators in the broad Games genre. They still do
    # not belong to this specifically games-only collection.
    if EMULATOR.search(name+' '+identifier) or re.search(r'\b(?:emulator|emulation)\b',desc) or name=='play!':return 'emulators'
    if re.search(r'\b(?:finder|launcher|guide|manual|executor|cheat|cheats|icreatepro|appleware|codex)\b',name) or 'launcher' in identifier or re.search(r'\b(?:game|edition) launcher\b',desc) or name=='megagames':return 'other'
    explicit=[]
    for key in ('category','categories','genre','genres','primaryGenreName','primaryGenreId'):
        value=app.get(key)
        if isinstance(value,list):explicit.extend(str(v).lower() for v in value[:32])
        elif value is not None:explicit.append(str(value).lower())
    if any(v in ('emulator','emulators','emulation') for v in explicit):return 'emulators'
    if any(v in ('game','games','gaming','6014') for v in explicit):return 'games'
    if explicit:return 'other'
    if KNOWN.search(name) or re.search(r'\b(?:a|an|the) (?:(?:classic|mobile|arcade|puzzle|racing|action|adventure|platform|role-playing|video|strategy) )+(?:game|platformer)\b',desc) or re.search(r'\b(?:a|an) (?:platformer|roguelike|rpg)\b',desc):return 'games'
    return 'other'
def inspect(item):
    index,url=item;report={'url':url,'counts':{'games':0,'emulators':0,'other':0,'invalid':0}}
    records=[]
    try:
        data,final=fetch(url);feed=json.loads(data)
        apps=feed.get('apps',feed.get('app')) if isinstance(feed,dict) else feed
        if not isinstance(apps,list):raise ValueError('Unsupported source: expected apps array')
        if len(apps)>20000:raise ValueError('Too many listings')
        report.update(name=feed.get('name') if isinstance(feed,dict) else None,final_url=final,bytes=len(data),sha256=hashlib.sha256(data).hexdigest())
        for app in apps:
            if not isinstance(app,dict):report['counts']['invalid']+=1;continue
            category=classify(app);report['counts'][category]+=1
            if category!='games':continue
            identifier=app.get('bundleIdentifier',app.get('bundleid'));name=app.get('name',app.get('appname'))
            if not isinstance(identifier,str) or not identifier or not isinstance(name,str):report['counts']['invalid']+=1;continue
            versions=app.get('versions') if 'versions' in app else [app]
            if not isinstance(versions,list):report['counts']['invalid']+=1;continue
            valid=[]
            for order,v in enumerate(versions[:100]):
                if not isinstance(v,dict):continue
                version=v.get('version');download=v.get('downloadURL',v.get('down'))
                if not isinstance(version,str) or not isinstance(download,str) or not https(download):continue
                output={k:v[k] for k in ('version','date','localizedDescription','size','minOSVersion','maxOSVersion','buildVersion') if k in v}
                output.update(version=version,downloadURL=download)
                valid.append((output,index,order,url))
            if not valid:report['counts']['invalid']+=1;continue
            metadata={'name':name[:256],'bundleIdentifier':identifier[:256],
                      'developerName':str(app.get('developerName',app.get('developer','Unknown')))[:256],
                      'localizedDescription':str(app.get('localizedDescription',app.get('description','')))[:4096],
                      'category':'Games'}
            icon=app.get('iconURL',app.get('icon'))
            if isinstance(icon,str) and https(icon):metadata['iconURL']=icon
            records.append((metadata,valid))
        report['error']=None
    except Exception as error:report['error']=str(error)
    return report,records
def published(version):
    try:
        value=datetime.datetime.fromisoformat(version.get('date','').replace('Z','+00:00'))
        if value.tzinfo is None:value=value.replace(tzinfo=datetime.timezone.utc)
        return value.timestamp()
    except (ValueError,TypeError):return float('-inf')
def numeric_version(version):
    value=version.get('version','').removeprefix('v')
    return tuple(int(n) for n in re.findall(r'\d+',value)) if re.match(r'^\d',value) else ()
def version_priority(record):
    return (published(record[0]),numeric_version(record[0]),-record[1],-record[2])
def build():
    raw,_=fetch(GIST);gist=json.loads(raw)
    content='\n'.join(fetch(file['raw_url'])[0].decode('utf-8') if file.get('truncated') else file['content'] for file in gist['files'].values())
    section=content.split('Repositories',1)[1].split('## 📲 Telegram',1)[0]
    urls=list(dict.fromkeys(re.findall(r'https://[^\s`<>]+',section)))
    sources=[];combined={};duplicates=0;collisions=[]
    with concurrent.futures.ThreadPoolExecutor(max_workers=10) as executor:
        for report,records in executor.map(inspect,enumerate(urls)):
            sources.append(report);print(report['url'],report.get('error') or report['counts'],flush=True)
            for metadata,versions in records:
                app=combined.setdefault(metadata['bundleIdentifier'],{'metadata':metadata,'versions':{}})
                for version,index,order,url in versions:
                    old=app['versions'].get(version['version'])
                    if old:
                        duplicates+=1
                        if len(collisions)<2000:collisions.append({'bundleIdentifier':metadata['bundleIdentifier'],'version':version['version'],
                            'chosen_source':url if published(version)>published(old[0]) else old[3],
                            'other_source':old[3] if published(version)>published(old[0]) else url})
                    if old is None or published(version)>published(old[0]):app['versions'][version['version']]=(version,index,order,url,metadata)
    apps=[]
    for app in combined.values():
        versions=sorted(app['versions'].values(),key=version_priority,reverse=True)
        metadata=dict(versions[0][4]);metadata['versions']=[v[0] for v in versions]
        metadata['playcoverSources']=list(dict.fromkeys(v[3] for v in versions))
        metadata['playcoverSelectedSource']=versions[0][3];apps.append(metadata)
    apps.sort(key=lambda a:a['name'].lower())
    feed={'name':'PlayCover-A Games Collection','identifier':'org.playcovera.games.collection',
          'subtitle':'Games from the supplied public source list',
          'description':'Combined public game listings. Sources remain responsible for downloads. Listing availability does not imply emulator compatibility.',
          'apps':apps,'news':[]}
    encoded=(json.dumps(feed,ensure_ascii=False,indent=2)+'\n').encode('utf-8')
    if len(encoded)>LIMIT:raise ValueError('Combined feed exceeds32MiB')
    Path('repositories').mkdir(exist_ok=True);Path('repositories/games.json').write_bytes(encoded)
    assets=Path('touchHLE-src/android/app/src/main/assets');assets.mkdir(exist_ok=True);(assets/'games.json').write_bytes(encoded)
    report={'gist':'https://gist.github.com/ongkiii/b40620d8d4a98ab17642858dce4cb2ec',
            'gist_revision':gist['history'][0]['version'],'generated_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'sources':sources,'source_count':len(sources),'game_count':len(apps),'duplicate_versions':duplicates,'deduplication_collisions':collisions,
            'output_bytes':len(encoded),'output_sha256':hashlib.sha256(encoded).hexdigest(),
            'classification':'Explicit game metadata first; otherwise conservative known title/description rules. Emulators and uncertain listings excluded.',
            'deduplication':'Bundle identifier and version; newest published timestamp wins. Version ordering uses published date, then numeric version, then source/version order.'}
    Path('repositories/games-report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print('Combined games:',len(apps),'sources:',len(sources),'bytes:',len(encoded))
if __name__=='__main__':build()
