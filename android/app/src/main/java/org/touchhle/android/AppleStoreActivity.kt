package org.touchhle.android

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.widget.*
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.Executors

class AppleStoreActivity:Activity(){
    companion object{
        private val workers=Executors.newFixedThreadPool(4)
        private val downloadWorkers=Executors.newCachedThreadPool()
        private val activeJobs=java.util.concurrent.ConcurrentHashMap<String,String>()
        private val activeApps=java.util.concurrent.ConcurrentHashMap.newKeySet<Long>()
    }
    private val main=Handler(Looper.getMainLooper())
    private val surfaceColor=Color.rgb(17,14,27);private val card=Color.rgb(33,27,49)
    private val accent=Color.rgb(184,143,255);private val muted=Color.rgb(178,169,194)
    private lateinit var content:LinearLayout;private lateinit var status:TextView;private lateinit var query:EditText
    private lateinit var more:Button;private lateinit var adapter:AppsAdapter
    private var endpoint:EditText?=null;private var token:EditText?=null
    private data class App(val id:Long,val owned:JSONObject?,var info:JSONObject?)
    private val apps=LinkedHashMap<Long,App>();private val rows=ArrayList<App>()
    private var generation=0;private var page=0;private var total=0;private var loading=false;private var ownedMode=true;private var endReached=false
    private fun dp(v:Int)=(v*resources.displayMetrics.density).toInt()
    private fun configFile()=File(noBackupFilesDir,"apple-store-companion.json")
    private fun rounded(color:Int,radius:Int=18)=GradientDrawable().apply{setColor(color);cornerRadius=dp(radius).toFloat()}
    private fun label(value:String,size:Float=15f,color:Int=Color.WHITE)=TextView(this).apply{text=value;textSize=size;setTextColor(color)}
    private fun button(value:String,action:()->Unit)=Button(this).apply{text=value;setTextColor(accent);background=rounded(card,12);isAllCaps=false;setOnClickListener{action()}}
    private fun addButton(value:String,action:()->Unit){content.addView(button(value,action),LinearLayout.LayoutParams(-1,dp(48)).apply{topMargin=dp(8)})}
    private fun alive(action:()->Unit){main.post{if(!isFinishing&&!isDestroyed)action()}}
    override fun onCreate(savedInstanceState:Bundle?){
        super.onCreate(savedInstanceState);window.statusBarColor=surfaceColor;window.navigationBarColor=surfaceColor
        content=LinearLayout(this).apply{orientation=LinearLayout.VERTICAL;setPadding(dp(20),dp(16),dp(20),dp(12));setBackgroundColor(surfaceColor)}
        if(intent.getBooleanExtra("settings",false)){setContentView(ScrollView(this).apply{addView(content)});settings();return}
        setContentView(content);content.addView(label("App Store",30f).apply{setTypeface(null,Typeface.BOLD)})
        content.addView(label("Your collection · Public catalog: US",14f,muted));status=label("Loading your purchases…",14f,muted)
        content.addView(status,LinearLayout.LayoutParams(-1,-2).apply{topMargin=dp(10);bottomMargin=dp(10)})
        val searchRow=LinearLayout(this);query=EditText(this).apply{hint="Find apps and games";setHintTextColor(muted);setTextColor(Color.WHITE);setSingleLine();setPadding(dp(14),0,dp(8),0);background=rounded(card,12)}
        searchRow.addView(query,LinearLayout.LayoutParams(0,dp(48),1f));searchRow.addView(button("Search"){search()},LinearLayout.LayoutParams(dp(92),dp(48)).apply{leftMargin=dp(8)});content.addView(searchRow)
        val tabs=LinearLayout(this);tabs.addView(button("Purchased"){purchases(true)},LinearLayout.LayoutParams(0,dp(46),1f))
        tabs.addView(button("Downloads"){AlertDialog.Builder(this).setTitle("Downloads").setMessage(activeJobs.values.joinToString("\n").ifEmpty{"No active downloads"}).setPositiveButton("Close",null).show()},LinearLayout.LayoutParams(0,dp(46),1f).apply{leftMargin=dp(8)})
        content.addView(tabs,LinearLayout.LayoutParams(-1,dp(46)).apply{topMargin=dp(10);bottomMargin=dp(10)})
        adapter=AppsAdapter();val list=ListView(this).apply{divider=null;cacheColorHint=surfaceColor;this.adapter=this@AppleStoreActivity.adapter;setOnItemClickListener{_,_,position,_->details(rows[position])}}
        content.addView(list,LinearLayout.LayoutParams(-1,0,1f));more=button("Load more purchases"){purchases(false)};more.visibility=View.GONE;content.addView(more,LinearLayout.LayoutParams(-1,dp(46)));addButton("Back to library"){finish()}
        val initial=++generation;workers.execute{try{val state=json(connection(),"/v1/status");alive{if(generation==initial){if(state.optBoolean("authenticated"))purchases(true)else status.text="Browse apps, or sign in through Settings → App Store account"}}}catch(_:Exception){alive{if(generation==initial)status.text="Public search is ready · account connection is in Settings"}}}
    }
    private fun settings(){
        content.addView(label("App Store account",28f).apply{setTypeface(null,Typeface.BOLD)});content.addView(label("Apple sign-in and 2FA happen on your computer. Connect to that account companion here.",15f,muted))
        endpoint=EditText(this).apply{hint="Companion URL";setTextColor(Color.WHITE);setHintTextColor(muted);setText("http://127.0.0.1:18765");inputType=InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI}
        token=EditText(this).apply{hint="Connection token";setTextColor(Color.WHITE);setHintTextColor(muted);inputType=InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD}
        content.addView(endpoint);content.addView(token);try{val c=JSONObject(configFile().readText());endpoint?.setText(c.getString("url"));token?.setText(c.getString("token"))}catch(_:Exception){}
        status=label("Connection information stays private",14f,muted);content.addView(status)
        addButton("Save connection"){try{val c=connection();configFile().writeText(JSONObject().put("url",c.first).put("token",c.second).toString());status.text="Connection saved"}catch(e:Exception){status.text=e.message}}
        addButton("Check account"){request("/v1/status"){status.text=when{it.optBoolean("authenticated")->"Signed in · purchased apps are ready";it.optBoolean("login_pending")->"Complete the computer sign-in prompt";else->"Sign-in or keychain unlock is required on the computer"}}}
        addButton("Sign in on computer"){request("/v1/login",JSONObject()){status.text=it.optString("message","Complete the computer prompt")}}
        addButton("Open App Store"){startActivity(Intent(this,AppleStoreActivity::class.java));finish()};addButton("Back to settings"){finish()}
    }
    private fun connection():Pair<String,String>{
        val saved=if(endpoint==null)JSONObject(configFile().readText())else null
        val base=(endpoint?.text?.toString()?:saved!!.getString("url")).trim().trimEnd('/');val uri=URI(base)
        require(uri.userInfo==null&&uri.query==null&&uri.fragment==null&&uri.path.isNullOrEmpty()){ "Use a connection origin without credentials or a path" }
        require(uri.scheme=="https"||(uri.scheme=="http"&&uri.host in listOf("127.0.0.1","localhost","::1"))){"Remote connections require HTTPS"}
        val secret=(token?.text?.toString()?:saved!!.getString("token")).trim();require(secret.length>=32){"Enter the connection token"};return base to secret
    }
    private fun open(c:Pair<String,String>,path:String):HttpURLConnection{require(path.startsWith("/v1/"));return(URL(c.first+path).openConnection()as HttpURLConnection).apply{connectTimeout=15000;readTimeout=120000;instanceFollowRedirects=false;setRequestProperty("Authorization","Bearer "+c.second)}}
    private fun json(c:Pair<String,String>,path:String,body:JSONObject?=null):JSONObject{val conn=open(c,path);try{if(body!=null){conn.requestMethod="POST";conn.doOutput=true;conn.setRequestProperty("Content-Type","application/json");conn.outputStream.use{it.write(body.toString().toByteArray())}};val code=conn.responseCode;val stream=if(code in 200..299)conn.inputStream else conn.errorStream;val obj=JSONObject(stream?.bufferedReader()?.use{it.readText()}?:"{}");require(code in 200..299){obj.optString("error","Companion request failed")};return obj}finally{conn.disconnect()}}
    private fun request(path:String,body:JSONObject?=null,complete:(JSONObject)->Unit){val c=try{connection()}catch(e:Exception){status.text=e.message;return};workers.execute{try{val obj=json(c,path,body);alive{complete(obj)}}catch(e:Exception){alive{status.text=e.message?:"Companion unavailable"}}}}
    private fun render(){rows.clear();rows.addAll(apps.values);adapter.notifyDataSetChanged();status.text=if(ownedMode)"${rows.size} of $total purchased apps" else "${rows.size} App Store results";more.visibility=if(ownedMode&&!endReached&&page*50<total)View.VISIBLE else View.GONE;more.isEnabled=!loading}
    private fun purchases(reset:Boolean){
        if(loading&&!reset)return;val c=try{connection()}catch(_:Exception){status.text="Sign in through Settings → App Store account to see purchases";return}
        if(reset){generation++;ownedMode=true;page=0;endReached=false};val current=generation;val requested=if(reset)1 else page+1;loading=true;more.isEnabled=false;status.text="Loading purchases…"
        workers.execute{try{val obj=json(c,"/v1/purchases?page=$requested");val source=obj.optJSONArray("apps")?:throw IllegalStateException("Purchase list unavailable")
            val added=(0 until source.length()).mapNotNull{source.optJSONObject(it)?.takeIf{a->a.optLong("id",a.optLong("trackId"))>0}}
            alive{if(generation==current&&ownedMode){if(reset)apps.clear();for(a in added){val id=a.optLong("id",a.optLong("trackId"));if(!apps.containsKey(id))apps[id]=App(id,a,null)};page=requested;total=obj.optInt("totalCount",apps.size);loading=false;endReached=added.isEmpty();render();if(endReached&&apps.size<total)status.text="${apps.size} of $total loaded · no further page returned"}}
            val metadata=try{AppleStoreCatalog.lookup(added.map{it.optLong("id",it.optLong("trackId"))})}catch(_:Exception){emptyMap<Long,JSONObject>()}
            alive{if(generation==current&&ownedMode){for((id,info)in metadata)apps[id]?.info=info;adapter.notifyDataSetChanged()}}
        }catch(e:Exception){alive{if(generation==current){loading=false;more.isEnabled=true;status.text=e.message?:"Purchases unavailable"}}}}
    }
    private fun search(){val term=query.text.toString().trim();if(term.isEmpty())return;val current=++generation;ownedMode=false;loading=false;more.visibility=View.GONE;status.text="Searching Apple’s catalog…";workers.execute{try{val found=AppleStoreCatalog.search(term);alive{if(generation==current&&!ownedMode){apps.clear();for(info in found){val id=info.getLong("trackId");apps[id]=App(id,null,info)};render()}}}catch(_:Exception){alive{if(generation==current)status.text="Apple catalog unavailable; try again shortly"}}}}
    private fun name(a:App)=a.info?.optString("trackName")?.takeIf{it.isNotBlank()}?:a.owned?.let{it.optString("name",it.optString("trackName"))}?.takeIf{it.isNotBlank()}?:"App ${a.id}"
    private fun image(view:ImageView,address:String,screenshot:Boolean=false){view.tag=address;view.setImageDrawable(rounded(Color.rgb(58,45,80),14));if(address.isBlank())return;workers.execute{try{val bitmap=AppleStoreCatalog.image(address,screenshot);alive{if(view.tag==address&&bitmap!=null)view.setImageBitmap(bitmap)}}catch(_:Exception){}}}
    private data class RowHolder(val icon:ImageView,val title:TextView,val subtitle:TextView,val rating:TextView,val version:TextView)
    private inner class AppsAdapter:BaseAdapter(){
        override fun getCount()=rows.size;override fun getItem(position:Int)=rows[position];override fun getItemId(position:Int)=rows[position].id
        override fun getView(position:Int,convertView:View?,parent:ViewGroup?):View{
            val a=rows[position]
            val wrapper=convertView as? LinearLayout ?: LinearLayout(this@AppleStoreActivity).apply{
            val row=LinearLayout(this@AppleStoreActivity).apply{orientation=LinearLayout.HORIZONTAL;gravity=android.view.Gravity.CENTER_VERTICAL;setPadding(dp(12),dp(12),dp(12),dp(12));background=rounded(card,16)}
            val icon=ImageView(this@AppleStoreActivity).apply{scaleType=ImageView.ScaleType.FIT_CENTER;clipToOutline=true;background=rounded(card,14)};row.addView(icon,LinearLayout.LayoutParams(dp(62),dp(62)))
            val info=LinearLayout(this@AppleStoreActivity).apply{orientation=LinearLayout.VERTICAL;setPadding(dp(14),0,dp(4),0)}
            val title=label("",17f).apply{setTypeface(null,Typeface.BOLD);maxLines=2};val subtitle=label("",12f,muted).apply{maxLines=2};val rating=label("",12f,accent);val version=label("",12f,muted)
            info.addView(title);info.addView(subtitle);info.addView(rating);info.addView(version);row.addView(info,LinearLayout.LayoutParams(0,-2,1f));setPadding(0,0,0,dp(8));addView(row,LinearLayout.LayoutParams(-1,-2));tag=RowHolder(icon,title,subtitle,rating,version)
            }
            val holder=wrapper.tag as RowHolder;holder.title.text=name(a)
            holder.subtitle.text=a.info?.let{it.optString("artistName")+" · "+it.optString("primaryGenreName")}?:"Purchased app · catalog details unavailable"
            val rating=a.info?.optDouble("averageUserRating",Double.NaN)?:Double.NaN
            holder.rating.text=if(rating.isNaN())a.owned?.optString("version")?.let{"Owned version $it"}?:"Tap for details" else "★ ${"%.1f".format(rating)} · "+a.info!!.optString("formattedPrice","")
            val ownedVersion=a.owned?.optString("version")?.takeIf{it.isNotBlank()};val storeVersion=a.info?.optString("version")?.takeIf{it.isNotBlank()}
            holder.version.text=listOfNotNull(ownedVersion?.let{"Owned $it"},storeVersion?.let{"Store $it"}).joinToString(" · ").ifEmpty{"Version unavailable"}
            image(holder.icon,a.info?.optString("artworkUrl100")?:"");return wrapper
        }
    }
    private fun details(a:App){
        val body=LinearLayout(this).apply{orientation=LinearLayout.VERTICAL;setPadding(dp(20),dp(16),dp(20),dp(16));setBackgroundColor(surfaceColor)}
        val heading=LinearLayout(this);val icon=ImageView(this).apply{scaleType=ImageView.ScaleType.FIT_CENTER};heading.addView(icon,LinearLayout.LayoutParams(dp(76),dp(76)));heading.addView(label(name(a),23f).apply{setTypeface(null,Typeface.BOLD);setPadding(dp(16),0,0,0)},LinearLayout.LayoutParams(0,-2,1f));body.addView(heading);image(icon,a.info?.optString("artworkUrl512",a.info?.optString("artworkUrl100")?:"")?:"")
        val info=a.info
        if(info==null)body.addView(label("Public details are not available in this catalog. The original purchase name and ID remain available.",15f,muted))else{
            val bytes=info.optString("fileSizeBytes").toLongOrNull();val size=bytes?.let{"%.1f MiB".format(it/(1024.0*1024.0))}?:"Unavailable";val rating=info.optDouble("averageUserRating",Double.NaN)
            val facts=listOf("Developer" to info.optString("artistName","Unavailable"),"Category" to info.optString("primaryGenreName","Unavailable"),"Rating" to if(rating.isNaN())"Unavailable" else "${"%.1f".format(rating)} (${info.optInt("userRatingCount")} ratings)","Current store version" to info.optString("version","Unavailable"),"Minimum iOS" to info.optString("minimumOsVersion","Unavailable"),"Size" to size)
            for((key,value)in facts)body.addView(label("$key · $value",14f,muted).apply{setPadding(0,dp(7),0,0)})
            body.addView(label(info.optString("description","Description unavailable"),15f).apply{setPadding(0,dp(18),0,dp(12))});info.optString("releaseNotes").takeIf{it.isNotBlank()}?.let{body.addView(label("What’s new",18f,accent));body.addView(label(it,14f,muted))}
            val shots=info.optJSONArray("screenshotUrls")?.takeIf{it.length()>0}?:info.optJSONArray("ipadScreenshotUrls")
            if(shots!=null&&shots.length()>0){body.addView(label("Screenshots",18f,accent));val rail=LinearLayout(this);for(i in 0 until minOf(shots.length(),8)){val shot=ImageView(this).apply{scaleType=ImageView.ScaleType.FIT_CENTER};rail.addView(shot,LinearLayout.LayoutParams(dp(180),dp(310)).apply{rightMargin=dp(10)});image(shot,shots.optString(i),true)};body.addView(HorizontalScrollView(this).apply{addView(rail)})}
            val address=info.optString("trackViewUrl");if(address.startsWith("https://"))body.addView(button("View on Apple App Store"){try{startActivity(Intent(Intent.ACTION_VIEW,android.net.Uri.parse(address)))}catch(_:Exception){}})
        }
        a.owned?.let{body.addView(label("Owned version ${it.optString("version","Unavailable")} · App ID ${a.id}",14f,accent))}
        body.addView(label("Downloads require an existing license. App Store IPAs may be encrypted or unsupported by this runtime.",13f,muted).apply{setPadding(0,dp(16),0,dp(12))});body.addView(button("Download owned app"){startDownload(a.id,name(a))})
        AlertDialog.Builder(this).setView(ScrollView(this).apply{addView(body)}).setPositiveButton("Close",null).show()
    }
    private fun startDownload(appId:Long,name:String){
        val c=try{connection()}catch(_:Exception){status.text="Connect your account through Settings → App Store account";return};if(!activeApps.add(appId)){status.text="$name is already downloading";return};val appContext=applicationContext
        downloadWorkers.execute{var id="";try{
            id=json(c,"/v1/downloads",JSONObject().put("app_id",appId)).getString("id");require(id.matches(Regex("[0-9a-f]{32}")));activeJobs[id]="$name: downloading";alive{status.text=activeJobs[id]}
            var done:JSONObject?=null;for(attempt in 0 until 1800){val job=json(c,"/v1/downloads/$id");when(job.getString("status")){"complete"->{done=job;break};"failed"->throw IllegalStateException(job.optString("error","Download failed"))};activeJobs[id]="$name: ${job.optLong("bytes")/(1024*1024)} MiB";Thread.sleep(2000)}
            val result=done?:throw IllegalStateException("Download still running on computer");val dir=File(appContext.getExternalFilesDir(null),"touchHLE_apps").apply{mkdirs()};val part=File(dir,"$id.apple-store-part");val target=File(dir,"$appId-$id.ipa");val digest=MessageDigest.getInstance("SHA-256");var count=0L;val conn=open(c,"/v1/files/$id")
            try{require(conn.responseCode==200);conn.inputStream.use{input->part.outputStream().use{out->val buffer=ByteArray(256*1024);while(true){val n=input.read(buffer);if(n<0)break;out.write(buffer,0,n);digest.update(buffer,0,n);count+=n}}}}finally{conn.disconnect()}
            val sha=digest.digest().joinToString(""){"%02x".format(it.toInt()and 255)};require(count==result.getLong("bytes")&&sha==result.getString("sha256")){"IPA transfer verification failed"};require(!target.exists()&&part.renameTo(target)){"IPA import failed"};activeJobs[id]="$name: imported into library";alive{status.text=activeJobs[id]}
        }catch(e:Exception){val error=e.message?:"Download failed";if(id.isNotEmpty())activeJobs[id]="$name: $error";alive{status.text=error}}finally{activeApps.remove(appId)}}
    }
    override fun onDestroy(){main.removeCallbacksAndMessages(null);super.onDestroy()}
}
