"""Read appdb's documented frontend public API and validate bounded source JSON."""
import concurrent.futures,datetime,hashlib,json,urllib.request,urllib.parse
from pathlib import Path
API='https://api.dbservices.to/v1.7/get_repos/?is_public=1&lang=en&brand=appdb'
LIMIT=32*1024*1024
class HTTPSRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,request,fp,code,msg,headers,newurl):
        if urllib.parse.urlsplit(newurl).scheme!='https':raise ValueError('Non-HTTPS redirect')
        return super().redirect_request(request,fp,code,msg,headers,newurl)
def get(url):
    parts=urllib.parse.urlsplit(url)
    if parts.scheme!='https' or parts.username or parts.password:raise ValueError('Invalid HTTPS source URL')
    opener=urllib.request.build_opener(HTTPSRedirects())
    request=urllib.request.Request(url,headers={'User-Agent':'Mozilla/5.0','Accept':'application/json'})
    with opener.open(request,timeout=20) as response:
        data=response.read(LIMIT+1)
        if len(data)>LIMIT:raise ValueError('Feed exceeds launcher 32MiB limit')
        return data,response.geturl()
def validate(entry):
    result={'name':entry['name'],'url':entry['url'],'appdb_id':entry['id'],
            'appdb_status':entry['status'],'appdb_cached_contents_url':entry['contents_uri']}
    try:
        data,final=get(entry['url']);feed=json.loads(data)
        if not isinstance(feed,dict) or not isinstance(feed.get('apps'),list) or not isinstance(feed.get('name'),str):
            raise ValueError('Expected AltStore source name and apps array')
        result.update(validation_error=None,final_url=final,feed_name=feed['name'],feed_identifier=feed.get('identifier'),
            app_count=len(feed['apps']),feed_bytes=len(data),feed_sha256=hashlib.sha256(data).hexdigest())
    except Exception as error:result['validation_error']=str(error)
    return result
raw,_=get(API);response=json.loads(raw)
assert response['success'] is True
Path('tools/appdb-api-repos.json').write_bytes(raw)
with concurrent.futures.ThreadPoolExecutor(max_workers=6) as executor:entries=list(executor.map(validate,response['data']))
unique=[];duplicates=[];urls=set();identities=set()
for entry in entries:
    # Same final HTTPS URL or byte-identical feed is a genuine duplicate; equal
    # human names alone do not prove sources have the same contents.
    keys={('url',entry.get('final_url',entry['url']))}
    if entry.get('feed_sha256'):keys.add(('sha256',entry['feed_sha256']))
    if entry['url'] in urls or keys & identities:duplicates.append(entry)
    else:unique.append(entry);urls.add(entry['url']);identities.update(keys)
report={'origin':'https://appdb.to/repos','metadata_api':API,
    'retrieved_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'listed_count':len(entries),'repositories':unique,'duplicates':duplicates,
    'validation_scope':'HTTPS fetch capped at32MiB; JSON name/apps schema only, no IPA downloads or runtime compatibility claim'}
Path('tools/appdb-repositories.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
print(json.dumps({'listed':len(entries),'unique':len(unique),'duplicates':len(duplicates),
    'errors':[(e['name'],e['validation_error']) for e in unique if e['validation_error']]}))
