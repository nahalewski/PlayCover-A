package org.touchhle.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

object AppleStoreCatalog {
    private val metadata=object:LinkedHashMap<Long,JSONObject>(256,.75f,true){override fun removeEldestEntry(eldest:MutableMap.MutableEntry<Long,JSONObject>?):Boolean=size>400}
    private val images=object:LruCache<String,Bitmap>(8*1024*1024){override fun sizeOf(key:String,value:Bitmap)=value.byteCount}
    private fun bytes(address:String,limit:Int):ByteArray{
        val url=URL(address);require(url.protocol=="https");val conn=url.openConnection()as HttpURLConnection
        conn.connectTimeout=15000;conn.readTimeout=20000;conn.instanceFollowRedirects=false
        try{require(conn.responseCode==200);return conn.inputStream.use{input->val out=java.io.ByteArrayOutputStream();val buffer=ByteArray(16384);while(true){val n=input.read(buffer);if(n<0)break;require(out.size()+n<=limit);out.write(buffer,0,n)};out.toByteArray()}}finally{conn.disconnect()}
    }
    private fun fetch(address:String)=JSONObject(String(bytes(address,4*1024*1024),Charsets.UTF_8))
    fun search(term:String):List<JSONObject>{
        require(term.isNotBlank()&&term.length<=200)
        val array=fetch("https://itunes.apple.com/search?entity=software&media=software&country=US&limit=50&term="+URLEncoder.encode(term,"UTF-8")).optJSONArray("results")?:return emptyList()
        return(0 until array.length()).mapNotNull{array.optJSONObject(it)?.takeIf{a->a.optLong("trackId")>0&&a.optString("kind")=="software"}}.also{values->synchronized(metadata){values.forEach{metadata[it.getLong("trackId")]=it}}}
    }
    fun lookup(ids:List<Long>):Map<Long,JSONObject>{
        val result=mutableMapOf<Long,JSONObject>();val missing=synchronized(metadata){ids.distinct().filter{id->metadata[id]?.let{result[id]=it;true}!=true}}
        for(batch in missing.chunked(50)){
            val array=fetch("https://itunes.apple.com/lookup?country=US&entity=software&id="+batch.joinToString(",")).optJSONArray("results")?:continue
            for(i in 0 until array.length()){val a=array.optJSONObject(i)?:continue;val id=a.optLong("trackId");if(id in batch&&a.optString("kind")=="software"){result[id]=a;synchronized(metadata){metadata[id]=a}}}
        };return result
    }
    fun image(address:String,screenshot:Boolean=false):Bitmap?{
        if(address.isBlank())return null;val key=address+if(screenshot)"/preview"else"/icon";synchronized(images){images.get(key)?.let{return it}}
        val data=bytes(address,8*1024*1024);val bounds=BitmapFactory.Options().apply{inJustDecodeBounds=true};BitmapFactory.decodeByteArray(data,0,data.size,bounds)
        if(bounds.outWidth<=0||bounds.outHeight<=0)return null;val max=if(screenshot)800 else 192;var sample=1;while(bounds.outWidth/sample>max||bounds.outHeight/sample>max)sample*=2
        val bitmap=BitmapFactory.decodeByteArray(data,0,data.size,BitmapFactory.Options().apply{inSampleSize=sample})?:return null;synchronized(images){images.put(key,bitmap)};return bitmap
    }
}
