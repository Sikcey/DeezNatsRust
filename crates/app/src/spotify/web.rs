//! Explicit browser session capture plus direct native request dispatch.
//! Routine requests never start or refresh WebView2 automatically.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;
use url::Url;
#[path = "web_native.rs"]
mod native;
pub(crate) use native::save_session;

pub(crate) struct Request {
    pub command: Value,
    pub reply: mpsc::SyncSender<Result<Value, String>>,
}
pub(crate) struct Transport {
    pub cookie: String,
    pub requests: mpsc::Receiver<Request>,
    pub cancel: Arc<AtomicBool>,
}
struct Worker {
    cookie: String,
    tx: mpsc::SyncSender<Request>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

fn cookie() -> Result<String, String> {
    let value = crate::session::provider_saved("spotify-web")
        .ok()
        .and_then(|s| saved_cookie(&s))
        .or_else(|| {
            crate::session::provider_saved("spotify")
                .ok()
                .and_then(|s| serde_json::from_str::<super::Credentials>(&s).ok())
                .and_then(|c| saved_cookie(&c.sp_dc))
        })
        .ok_or("Sign in to Spotify with the Web Player first")?;
    if !crate::spotify_webview::valid_spotify_cookie("sp_dc", ".spotify.com", true, &value) {
        return Err("Spotify web session is invalid; sign in again".into());
    }
    Ok(value)
}
fn saved_cookie(saved: &str) -> Option<String> {
    let decoded = serde_json::from_str::<Value>(saved).ok();
    let value = decoded
        .as_ref()
        .and_then(|v| v.as_str().or_else(|| v["sp_dc"].as_str()))
        .unwrap_or(saved);
    crate::spotify_webview::valid_spotify_cookie("sp_dc", ".spotify.com", true, value)
        .then(|| value.to_owned())
}
/// Presence only; the next request verifies the grant with Spotify.
pub(crate) fn has_session() -> bool {
    cookie().is_ok()
}
pub(crate) fn reset() {
    native::reset();
    if let Ok(mut worker) = WORKER.lock() {
        *worker = None;
    }
}
static REFRESHING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn refreshing() -> bool {
    REFRESHING.load(Ordering::Relaxed) != 0
}

pub(crate) struct RefreshGuard;
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        REFRESHING.fetch_sub(1, Ordering::Relaxed);
    }
}
pub(crate) fn begin_refresh() -> RefreshGuard {
    REFRESHING.fetch_add(1, Ordering::Relaxed);
    RefreshGuard
}
pub(crate) fn refresh() -> Result<(), String> {
    let _guard = begin_refresh();
    let result = capture_session();
    if let Ok(mut worker) = WORKER.lock() {
        *worker = None;
    }
    save_session(result?)?;
    Ok(())
}
pub(crate) fn account() -> Result<Value, String> {
    let value = query("profileAttributes", json!({}))?;
    let profile = value
        .pointer("/data/me/profile")
        .filter(|p| p.is_object())
        .ok_or("Spotify web session has no authenticated profile; sign in again")?;
    let username = profile["username"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("Spotify web session has no account identity; sign in again")?;
    Ok(
        json!({"id":username,"userId":username,"uri":profile["uri"],"display_name":profile["name"],"name":profile["name"],"images":profile["avatar"]["sources"],"provider":"spotify"}),
    )
}
fn capture_session() -> Result<Value, String> {
    let command = json!({"kind":"export"});
    let saved = cookie()?;
    let mut worker = WORKER
        .lock()
        .map_err(|_| "Spotify web transport unavailable")?;
    if worker
        .as_ref()
        .is_none_or(|w| w.cookie != saved || w.cancel.load(Ordering::Relaxed))
    {
        *worker = None;
        let (tx, requests) = mpsc::sync_channel(16);
        let cancel = Arc::new(AtomicBool::new(false));
        let transport = Transport {
            cookie: saved.clone(),
            requests,
            cancel: cancel.clone(),
        };
        let stopped = cancel.clone();
        std::thread::Builder::new()
            .name("spotify-web-session".into())
            .spawn(move || {
                let result = crate::spotify_webview::transport(transport);
                // Never log browser results, account payloads, or credentials.
                let _ = result;
                stopped.store(true, Ordering::Relaxed);
            })
            .map_err(|_| "Could not start Spotify WebView2 session")?;
        *worker = Some(Worker {
            cookie: saved,
            tx,
            cancel,
        });
    }
    let (reply, rx) = mpsc::sync_channel(1);
    let tx = worker.as_ref().unwrap().tx.clone();
    let cancel = worker.as_ref().unwrap().cancel.clone();
    drop(worker);
    tx.try_send(Request { command, reply })
        .map_err(|_| "Spotify web session is busy or closed")?;
    let started = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Spotify web request cancelled or session closed".into());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(
                    "Spotify WebView2 session unavailable; update the Runtime or sign in again"
                        .into(),
                )
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if started.elapsed() >= Duration::from_secs(95) {
            cancel.store(true, Ordering::Relaxed);
            return Err("Spotify web session timed out; retry or sign in again".into());
        }
    }
}
// Only replay read-only operations after refreshing an expired web session.
// A single refresh is shared by concurrent catalog requests.
static AUTO_REFRESH: Mutex<()> = Mutex::new(());
fn execute(command: Value) -> Result<Value, String> {
    let result = native::execute(command.clone());
    let read_only = (matches!(command["kind"].as_str(), Some("query" | "browse"))
        && command["mutation"] != true)
        || (command["kind"] == "rest" && command["method"] == "GET");
    if !read_only
        || !result
            .as_ref()
            .is_err_and(|e| e == native::REFRESH_REQUIRED)
    {
        return result;
    }
    let _guard = AUTO_REFRESH
        .lock()
        .map_err(|_| "Spotify session refresh unavailable")?;
    let retry = native::execute(command.clone());
    if !retry.as_ref().is_err_and(|e| e == native::REFRESH_REQUIRED) {
        return retry;
    }
    refresh().map_err(|_| "Spotify session could not be refreshed. Sign in again.")?;
    native::execute(command)
}
pub(crate) fn query(operation: &str, variables: Value) -> Result<Value, String> {
    query_inner(operation, variables, false)
}
pub(crate) fn mutation(operation: &str, variables: Value) -> Result<Value, String> {
    query_inner(operation, variables, true)
}
fn query_inner(operation: &str, variables: Value, mutation: bool) -> Result<Value, String> {
    if operation.is_empty()
        || operation.len() > 100
        || !operation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("Invalid Spotify web operation".into());
    }
    if variables.to_string().len() > 512 * 1024 {
        return Err("Spotify request is too large".into());
    }
    execute(
        json!({"kind":"query", "operation":operation, "variables":variables, "mutation":mutation}),
    )
}
pub(crate) fn browse() -> Result<Value, String> {
    execute(json!({"kind":"browse"}))
}
pub(crate) fn browse_operations() -> Result<Vec<String>, String> {
    let result = native::browse_operations();
    if !result
        .as_ref()
        .is_err_and(|e| e == native::REFRESH_REQUIRED)
    {
        return result;
    }
    let _guard = AUTO_REFRESH
        .lock()
        .map_err(|_| "Spotify session refresh unavailable")?;
    let retry = native::browse_operations();
    if !retry.as_ref().is_err_and(|e| e == native::REFRESH_REQUIRED) {
        return retry;
    }
    refresh().map_err(|_| "Spotify session could not be refreshed. Sign in again.")?;
    native::browse_operations()
}
pub(super) fn directory_cache_scope() -> Result<u64, String> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cookie()?.hash(&mut hasher);
    Ok(hasher.finish())
}
// Device commands use native HTTP without starting a browser worker.
pub(crate) fn device_command(command: Value) -> Result<Value, String> {
    execute(json!({"kind":"devices", "command":command}))
}
fn approved_url(address: &str) -> Result<Url, String> {
    let url = Url::parse(address).map_err(|_| "Invalid Spotify web URL")?;
    let approved = match url.host_str() {
        Some("api.spotify.com") => {
            url.path().starts_with("/v1/")
                && (!url.path().starts_with("/v1/me/player")
                    || url.path() == "/v1/me/player/recently-played")
        }
        Some("spclient.wg.spotify.com") => [
            "/metadata/",
            "/user-profile-view/",
            "/recently-played/",
            "/inspiredby-mix/",
            "/connect-state/",
            "/player/",
            "/track-playback/",
            "/collection/",
            "/playlist/",
            "/color-lyrics/",
        ]
        .iter()
        .any(|p| url.path().starts_with(p)),
        _ => false,
    };
    if !approved
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || address.len() > 8192
    {
        return Err("Spotify web URL is outside the approved host/path allowlist".into());
    }
    Ok(url)
}
pub(crate) fn rest_get(url: &str) -> Result<Value, String> {
    rest_request("GET", url, None)
}
pub(crate) fn rest_request(method: &str, url: &str, body: Option<&Value>) -> Result<Value, String> {
    approved_url(url)?;
    if !matches!(method, "GET" | "POST" | "PUT" | "DELETE") || (method == "GET" && body.is_some()) {
        return Err("Invalid Spotify web request method/body".into());
    }
    if body.is_some_and(|v| v.to_string().len() > 512 * 1024) {
        return Err("Spotify request is too large".into());
    }
    execute(json!({"kind":"rest", "method":method,"url":url,"body":body}))
}
pub(crate) fn rest_upload(url: &str, body: &[u8], content_type: &str) -> Result<Value, String> {
    let parsed = approved_url(url)?;
    if content_type != "image/jpeg"
        || body.len() > 256 * 1024
        || body.len() < 3
        || !body.starts_with(&[0xff, 0xd8, 0xff])
        || parsed.host_str() != Some("api.spotify.com")
        || !parsed.path().starts_with("/v1/playlists/")
        || !parsed.path().ends_with("/images")
    {
        return Err("Spotify playlist cover must be a JPEG of at most 256 KiB".into());
    }
    execute(
        json!({"kind":"upload", "method":"PUT", "url":url,"body":STANDARD.encode(body),"contentType":content_type}),
    )
}

// Installed before Spotify's scripts. Tokens remain inside the private browser;
// native callers receive only the requested API result. The original fetch is
// retained so our replay does not recursively intercept itself.
pub(crate) const BOOTSTRAP: &str = r#"
(() => {
  if (location.origin !== 'https://open.spotify.com' || window !== window.top) return;
  const nativeFetch = window.fetch.bind(window);
const s = window.__deeznatsSpotifyWeb = {token:'', client:'', version:'', expiry:0, anonymous:true, hashes:{}, scanned:new Set(), cooldowns:{}, browse:null, result:null};
  const authHost = u => ['api-partner.spotify.com','api.spotify.com','spclient.wg.spotify.com'].includes(u.hostname) && u.protocol === 'https:' && !u.port;
  // Protocol verified in the current Web Player ConnectAPIClient. Observe only
  // browser-owned registrations/dealer traffic; never create another session.
  s.connect={sender:'',cluster:null};
  const deviceId=x=>typeof x==='string' && /^[A-Za-z0-9_%:-]{1,256}$/.test(x);
  function cluster(v) {
    const c=v?.cluster || v;
    if(!c || typeof c.devices!=='object' || !c.devices || Array.isArray(c.devices))return;
    const old=Number(s.connect.cluster?.timestamp), next=Number(c.timestamp);
    if(Number.isFinite(old) && Number.isFinite(next) && next<old)return;
    s.connect.cluster=c;
  }
  const Socket=window.WebSocket;
  window.WebSocket=new Proxy(Socket,{construct(target,args){
    const socket=new target(...args);
    let u;try{u=new URL(args[0]);}catch(_){return socket;}
    if(u.protocol==='wss:' && /(^|\.)dealer\.spotify\.com$/.test(u.hostname))socket.addEventListener('message',event=>{
      if(typeof event.data!=='string' || event.data.length>4*1024*1024)return;
      try {const v=JSON.parse(event.data);if(/\/connect-state\/v1\/cluster$/.test(v.uri||'')) {
              for(let p of v.payloads||[]) {if(typeof p==='string')p=JSON.parse(p);cluster(p?.cluster||p);}
      }}catch(_){}
    });
    return socket;
  }});
  function remember(body) {
    try { if (typeof body === 'string') body=JSON.parse(body);
      const h=body?.extensions?.persistedQuery?.sha256Hash;
      if (/^[a-f0-9]{64}$/.test(h) && /^[A-Za-z0-9_]{1,100}$/.test(body.operationName)) s.hashes[body.operationName]=h;
    } catch (_) {}
  }
  window.fetch = function(input, init) {
    let u; try { u=new URL(typeof input==='string' || input instanceof URL ? input : input.url,location.href); } catch (_) { return nativeFetch(input,init); }
    if (authHost(u)) {
      const headers=new Headers(input instanceof Request ? input.headers : undefined);
      new Headers(init?.headers).forEach((v,k)=>headers.set(k,v));
      const token=headers.get('authorization'); if (token?.startsWith('Bearer ')) s.token=token.slice(7);
      s.client=headers.get('client-token') || s.client;
      s.version=headers.get('spotify-app-version') || s.version;
      if (u.hostname==='api-partner.spotify.com') {
        if (init?.body) remember(init.body);
        if (input instanceof Request && !init?.body) input.clone().text().then(remember).catch(()=>{});
        if (u.searchParams.has('operationName')) remember({operationName:u.searchParams.get('operationName'),extensions:JSON.parse(u.searchParams.get('extensions')||'{}')});
      }
    }
    // Clone registration requests before nativeFetch consumes their body.
    const registrationBody=u.hostname==='spclient.wg.spotify.com' && u.pathname==='/track-playback/v1/devices' && input instanceof Request ? input.clone().text() : null;
    const response=nativeFetch(input,init);
    if(u.hostname==='spclient.wg.spotify.com') {
      // The sender is the local descriptor, not the hobs_ observer ID (which
      // Spotify truncates). Capture it from actual track-player registration.
      if(u.pathname==='/track-playback/v1/devices') {
        const capture=body=>{try{const v=JSON.parse(body);const id=v?.device?.device_info?.device_id||v?.device?.device_id;if(deviceId(id))s.connect.sender=id;}catch(_){}};
        if(typeof init?.body==='string')capture(init.body);
        else if(registrationBody)registrationBody.then(capture).catch(()=>{});
      }
      const registration=u.pathname.match(/^\/connect-state\/v1\/devices\/([^/]+)$/);
      const method=(init?.method||(input instanceof Request?input.method:'GET')).toUpperCase();
      // hobs_ registrations prove only the observer ID; the source truncates it.
      if(registration && method==='PUT' && deviceId(registration[1]) && !registration[1].startsWith('hobs_'))s.connect.sender=registration[1];
      const from=u.pathname.match(/^\/connect-state\/v1\/(?:connect\/[^/]+|player\/command)\/from\/([^/]+)\/to\//);
      if(from && deviceId(from[1]))s.connect.sender=from[1];
      if(/^\/connect-state\/v1\/(cluster|devices\/[^/]+)$/.test(u.pathname))response.then(async r=>{
        if(r.ok && r.status!==204){const v=JSON.parse(await limitedText(r.clone(),4*1024*1024));cluster(v?.cluster||v);}
      }).catch(()=>{});
    }
if(u.hostname==='api-partner.spotify.com') response.then(r=>r.clone().json()).then(v=>{
 if((v?.data?.browse || v?.data?.browseStart) && JSON.stringify(v).length<=4*1024*1024)s.browse=v;
}).catch(()=>{});
    if ((u.origin===location.origin && u.pathname==='/api/token') || u.hostname==='clienttoken.spotify.com') {
      response.then(r=>r.clone().json()).then(v=> {
        if(u.hostname==='clienttoken.spotify.com') s.client=v?.granted_token?.token || s.client;
        else if(v.accessToken) { s.token=v.accessToken; s.expiry=v.accessTokenExpirationTimestampMs||0; s.anonymous=v.isAnonymous !== false; }
      }).catch(()=>{});
    }
    return response;
  };
  const allowedAsset = x => { try { const u=new URL(x,location.href); return u.protocol==='https:' && !u.port && ['open.spotify.com','open.spotifycdn.com'].includes(u.hostname) && u.pathname.endsWith('.js'); } catch(_){return false;} };
  let runtime;
  function chunkURLs() {
    const out=[...document.scripts].map(x=>x.src).concat(performance.getEntriesByType('resource').map(x=>x.name)).filter(allowedAsset);
    if(!runtime) for(const key of Object.keys(window).filter(k=>k.startsWith('webpackChunk') || k==='rspackChunk')) {
      const chunks=window[key]; if(Array.isArray(chunks)) try { chunks.push([['deeznats_'+Date.now()],{},r=>{runtime=r;}]); } catch(_) {}
      if(runtime) break;
    }
    if(runtime?.u) {
      const ids=new Set([...runtime.u.toString().matchAll(/\b(\d+):/g)].map(m=>Number(m[1])));
      for(const id of ids) try { const u=new URL(runtime.p+runtime.u(id),location.href).href; if(allowedAsset(u))out.push(u); } catch(_){}
    }
    return [...new Set(out)].sort((a,b)=>Number(/vendor|locales|moment/.test(a))-Number(/vendor|locales|moment/.test(b)));
  }
  function scan(text) {
    for(const m of text.matchAll(/["']([A-Za-z][A-Za-z0-9_]{0,99})["']\s*,\s*["'](?:query|mutation)["']\s*,\s*["']([a-f0-9]{64})["']/g)) s.hashes[m[1]]=m[2];
    for(const m of text.matchAll(/operationName\s*:\s*["']([A-Za-z][A-Za-z0-9_]{0,99})["'][\s\S]{0,300}?sha256Hash\s*:\s*["']([a-f0-9]{64})["']/g))s.hashes[m[1]]=m[2];
  }
  async function limitedText(r,max) {
    if(!r.ok || Number(r.headers.get('content-length'))>max) throw Error('Spotify web response exceeded its limit or failed');
    const reader=r.body.getReader(); let size=0, parts=[];
    try { for(;;) { const {value,done}=await reader.read(); if(done)break; size+=value.byteLength; if(size>max)throw Error('Spotify web response exceeded its limit'); parts.push(value); } }
    finally { await reader.cancel().catch(()=>{}); }
    const bytes=new Uint8Array(size); let at=0; for(const p of parts){bytes.set(p,at);at+=p.length;} return new TextDecoder().decode(bytes);
  }
  async function discover(op,force=false) {
    if(force){ delete s.hashes[op]; s.scanned.clear(); }
    const deadline=Date.now()+25000; let count=0;
    // Loaded source first, then the active webpack runtime's current lazy chunks.
    const urls=chunkURLs();
    for(let at=0; at<urls.length && count<96 && Date.now()<deadline && !s.hashes[op];at+=4) {
      await Promise.all(urls.slice(at,at+4).map(async url=>{
        if(s.scanned.has(url))return; s.scanned.add(url);count++;
        try { const r=await nativeFetch(url,{credentials:'omit',redirect:'error',signal:AbortSignal.timeout(5000)}); scan(await limitedText(r,6*1024*1024)); } catch(_){}
      }));
    }
    return s.hashes[op];
  }
  s.ready=()=>!!s.token && !!s.client && !!s.version && s.anonymous===false && (!s.expiry || Date.now()+30000<s.expiry);
  s.run=async cmd=> {
    s.result=null;
if(cmd.kind==='browse') {
 const deadline=Date.now()+20000;
 while(!s.browse && Date.now()<deadline)await new Promise(r=>setTimeout(r,100));
 if(!s.browse)throw Error('Spotify search page did not provide browse categories; retry after refreshing');
 s.result={ok:true,value:s.browse};return;
}
    try {
      if(!s.ready()) { s.result={refresh:true};return; }
      if(cmd.kind==='export') {
        await discover('__deeznats_capture_all__');
const browseDeadline=Date.now()+20000;
while(!s.browse && Date.now()<browseDeadline)await new Promise(r=>setTimeout(r,100));
// Header-only captures have no server expiry. Cache them for at most five
// minutes; native requests still validate authorization and invalidate on 401.
s.result={ok:true,value:{token:s.token,client:s.client,version:s.version,expiry:s.expiry||Date.now()+300000,
hashes:s.hashes,browse:s.browse,userAgent:navigator.userAgent,sender:s.connect.sender}};
        return;
      }
      let url=cmd.url, method=cmd.method, body, mutation=false;
      if(cmd.kind==='devices') {
        const c=cmd.command;
        // Always fetch the current account's cluster. Cached dealer state is
        // advisory only and never used to invent an active remote device.
        const request=async (path,method='GET',payload)=>{
          const origin='https://spclient.wg.spotify.com';
          if(s.cooldowns[origin]>Date.now())throw Error('Spotify Connect is rate limited; retry later');
          const r=await nativeFetch(origin+'/connect-state/v1/'+path,{method,headers:{Authorization:'Bearer '+s.token,'Client-Token':s.client,'Spotify-App-Version':s.version,'App-Platform':'WebPlayer','Content-Type':'application/json'},body:payload===undefined?undefined:JSON.stringify(payload),credentials:'omit',redirect:'error',signal:AbortSignal.timeout(15000)});
          if(r.status===429)s.cooldowns[origin]=Date.now()+Math.max(1,Math.min(3600,Number(r.headers.get('retry-after'))||5))*1000;
          if(r.status===401)throw Object.assign(Error('Spotify Connect authorization expired'),{refresh:true});
          if(!r.ok)throw Error('Spotify Connect request failed (HTTP '+r.status+')');
          if(!r.body || r.status===204 || r.headers.get('content-length')==='0')return {status:r.status,value:null};
          const raw=await limitedText(r,4*1024*1024);return {status:r.status,value:raw.trim()?JSON.parse(raw):null};
        };
        const read=async()=>{const r=await request('cluster');const v=r.status===204 || r.value===null?{devices:{}}:(r.value?.cluster||r.value);
          // Proto JSON omits the empty device map for a genuine idle cluster.
          if(v && v.devices===undefined && !v.active_device_id && v.timestamp!==undefined && Number.isFinite(Number(v.timestamp)) && v.player_state && typeof v.player_state==='object' && !Array.isArray(v.player_state) && typeof v.need_full_player_state==='boolean')v.devices={};
          if(!v?.devices || typeof v.devices!=='object' || Array.isArray(v.devices))throw Error('Spotify Connect returned malformed device state');
          cluster(v);return v;};
        const state=await read();
        if(c.action==='state'){s.result={ok:true,value:{...state,deeznats_web_device_id:s.connect.sender}};return;}
        const sender=s.connect.sender;
        if(!deviceId(sender))throw Error('Spotify Web Player has not supplied its Connect sender; open Web Player and retry');
        const id=c.action==='transfer'?c.device:state.active_device_id;
        if(!deviceId(id) || !state.devices[id])throw Error('Spotify Connect target is unavailable');
        if(id===sender)throw Error('The hidden Spotify web session is for catalog access. Pair DeezNats in Spotify for local audio, or select another Spotify Connect device.');
        if(c.expected?.device && c.expected.device!==id)throw Error('Spotify active device changed; refresh before controlling playback');
        if(c.expected?.uri && c.expected.uri!==state.player_state?.track?.uri)throw Error('Spotify current item changed; refresh before editing queue');
        const d=state.devices[id], cap=d.capabilities||{};
        if(cap.is_controllable!==true)throw Error('Spotify Connect target does not allow remote control');
        let path, payload, verb='POST';
        if(c.action==='transfer'){
          if(d.can_play!==true || cap.hidden===true || d.is_restricted===true)throw Error('Spotify Connect target cannot receive playback');
          if(state.player_state?.restrictions?.disallow_transferring_playback_reasons?.length)throw Error('Spotify Connect transfer is restricted');
          path='connect/transfer';payload={transfer_options:{restore_paused:c.playing?'resume':'pause'},command_id:globalThis.crypto.randomUUID().replaceAll('-','')};
        }else if(c.action==='volume'){
          if(cap.disable_volume===true || !Number.isInteger(c.value) || c.value<0 || c.value>100)throw Error('Spotify Connect volume is unavailable');
          verb='PUT';path='connect/volume';payload={volume:Math.round(c.value*65535/100)};
        }else{
          const endpoints={pause:'pause',resume:'resume',next:'skip_next',previous:'skip_prev',seek:'seek_to',play:'play',queue:'set_queue',shuffle:'set_options',repeat:'set_options'};
          const endpoint=endpoints[c.action];if(!endpoint)throw Error('Unsupported Spotify Connect control');
          const restrictions=state.player_state?.restrictions||{};
          const keys={pause:'pausing',resume:'resuming',next:'skipping_next',previous:'skipping_prev',seek:'seeking',play:'interrupting_playback',queue:'set_queue',shuffle:'toggling_shuffle'};
          if(restrictions['disallow_'+keys[c.action]+'_reasons']?.length)throw Error('Spotify Connect control is restricted');
          payload={command:{endpoint}};path='player/command';
          if(c.action==='seek') {if(!Number.isSafeInteger(c.value)||c.value<0)throw Error('Invalid Spotify Connect seek');payload.command.value=c.value;}
          if(c.action==='play') {
            if(!c.descriptor?.context || !c.descriptor?.options)throw Error('Invalid Spotify Connect play descriptor');
            Object.assign(payload.command,c.descriptor,{endpoint});
          }
          if(c.action==='queue') {
            const p=state.player_state;
            if(p?.queue_revision==null || !Array.isArray(c.next_tracks) || (p.prev_tracks!=null && !Array.isArray(p.prev_tracks)))throw Error('Spotify has not supplied a replaceable queue revision');
            if(c.expected?.revision!=null && c.expected.revision!==p.queue_revision)throw Error('Spotify queue changed; refresh before editing');
            Object.assign(payload.command,{next_tracks:c.next_tracks,prev_tracks:p.prev_tracks||[],queue_revision:p.queue_revision});
          }
          if(c.action==='shuffle') {
            if(c.value!==0 && c.value!==1)throw Error('Invalid Spotify shuffle option');
            payload.command.shuffling_context=!!c.value;
          }
          if(c.action==='repeat') {
            if(![0,1,2].includes(c.value))throw Error('Invalid Spotify repeat option');
            if(restrictions.disallow_toggling_repeat_context_reasons?.length || restrictions.disallow_toggling_repeat_track_reasons?.length)throw Error('Spotify repeat control is restricted');
            Object.assign(payload.command,{repeating_context:c.value===1,repeating_track:c.value===2});
          }
          if(globalThis.crypto?.randomUUID)payload.command.logging_params={command_id:globalThis.crypto.randomUUID()};
        }
        const r=await request(path+'/from/'+encodeURIComponent(sender)+'/to/'+encodeURIComponent(id),verb,payload);
        if(!((r.status===200 && r.value?.ack_id) || (r.status===202 && !r.value?.ack_id)))throw Error('Spotify Connect did not acknowledge the command');
        if(c.action==='transfer') {
          // An acknowledgement alone is not evidence of a successful transfer.
          for(let i=0;i<5;i++) {await new Promise(r=>setTimeout(r,300));const v=await read();
            if(v.active_device_id===id && typeof v.player_state?.is_paused==='boolean' && v.player_state.is_paused===!c.playing){s.result={ok:true,value:null};return;}}
          throw Error('Spotify Connect accepted transfer but has not confirmed the target state; refresh devices');
        }
        s.result={ok:true,value:null};return;
      }
      if(cmd.kind==='query') {
        const hash=await discover(cmd.operation);
        if(!hash) throw Error('Spotify Web Player operation is unavailable in the current deployment: '+cmd.operation);
        url='https://api-partner.spotify.com/pathfinder/v2/query'; method='POST';
        body=JSON.stringify({operationName:cmd.operation,variables:cmd.variables,extensions:{persistedQuery:{version:1,sha256Hash:hash}}});
        mutation=/^(add|remove|create|update|delete|change|set|follow|unfollow)/i.test(cmd.operation);
      } else body=cmd.kind==='upload' ? cmd.body : cmd.body==null ? undefined : JSON.stringify(cmd.body);
      const u=new URL(url); if(!authHost(u)||u.username||u.password||u.hash)throw Error('Spotify web URL rejected');
if(s.cooldowns[u.origin]>Date.now())throw Error('Spotify web rate limited; retry after '+Math.ceil((s.cooldowns[u.origin]-Date.now())/1000)+' seconds');
      for(let attempt=0;attempt<2;attempt++) {
        const headers={'Authorization':'Bearer '+s.token,'Client-Token':s.client,'Spotify-App-Version':s.version,'App-Platform':'WebPlayer','Accept':'application/json'};
        if(body!==undefined)headers['Content-Type']=cmd.contentType||'application/json;charset=UTF-8';
        const r=await nativeFetch(url,{method,headers,body,credentials:'omit',redirect:'error',signal:AbortSignal.timeout(15000)});
        if(r.status===401 || (r.status===400 && r.headers.get('client-token-error')==='INVALID_CLIENTTOKEN')) { s.result={refresh:true};return; }
if(r.status===429) { const wait=Math.max(1,Math.min(3600,Number(r.headers.get('retry-after'))||5)); s.cooldowns[u.origin]=Date.now()+wait*1000; throw Error('Spotify web rate limited; retry after '+wait+' seconds'); }
        if(r.status>=500 && attempt===0 && (method==='GET'||(cmd.kind==='query'&&!mutation))) { await new Promise(r=>setTimeout(r,400));continue; }
        if(!r.ok)throw Error(r.status===403 ? 'Spotify web request is forbidden for this account (HTTP 403)' : 'Spotify web request failed (HTTP '+r.status+')');
        if(r.status===204 || r.headers.get('content-length')==='0') {s.result={ok:true,value:null};return;}
        const raw=await limitedText(r,4*1024*1024); const value=raw.trim()?JSON.parse(raw):null;
        if(value?.errors?.length) {
          if(attempt===0 && value.errors.every(e=>/persistedquerynotfound|persistedquerynotsupported|unknown operation/i.test((e.message||'').replace(/\s/g,'')))) {
            const hash=await discover(cmd.operation,true);if(!hash)throw Error('Spotify web operation changed; retry after signing in again');
            const payload=JSON.parse(body);payload.extensions.persistedQuery.sha256Hash=hash;body=JSON.stringify(payload);continue;
          }
          if(value.errors.some(e=>/unauthenticated|token expired/i.test(e.message||''))){s.result={refresh:true};return;}
          throw Error('Spotify Web Player rejected operation '+cmd.operation);
        }
        s.result={ok:true,value};return;
      }
      throw Error('Spotify web request could not be completed');
    } catch(e) { s.result=e?.refresh ? {refresh:true} : {ok:false,error: e?.name==='TimeoutError' ? 'Spotify web request timed out' : String(e.message||'Spotify web request failed')}; }
  };
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_only_go_to_approved_https_services() {
        for u in [
            "http://api.spotify.com/v1/me",
            "https://api.spotify.com.evil/v1/me",
            "https://user@api.spotify.com/v1/me",
            "https://api.spotify.com:444/v1/me",
            "https://spclient.wg.spotify.com/redirect/",
            "https://api.spotify.com/v1/me#secret",
            "https://open.spotify.com/v1/me",
        ] {
            assert!(approved_url(u).is_err());
        }
        for u in ["https://api.spotify.com/v1/me", "https://api.spotify.com/v1/me/player/recently-played", "https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/spotify:track:abc?response-format=json", "https://spclient.wg.spotify.com/recently-played/v3/user/u/recently-played?format=json"] { assert!(approved_url(u).is_ok()); }
        for path in ["", "/play", "/pause", "/queue", "/devices"] {
            assert!(approved_url(&format!("https://api.spotify.com/v1/me/player{path}")).is_err());
        }
    }
}
