use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{sleep, Duration};

use crate::state::{AppState, SubtitleStyle};

pub fn spawn(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let preferred_port =
            crate::state::normalize_server_port(state.config.read().await.server_port);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        if let Ok(mut current) = state.server_stop.lock() {
            *current = Some(stop_tx);
        }

        // Give restart_server a chance to cancel a task that has not bound its
        // socket yet. This avoids the old task winning a port-change race.
        if stop_rx.try_recv().is_ok() {
            return;
        }

        let address = SocketAddr::from(([0, 0, 0, 0], preferred_port));
        let listener = match TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("字幕端口 {preferred_port} 不可用，尝试随机局域网端口：{error}");
                match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        eprintln!("字幕服务启动失败：{error}");
                        state
                            .server_started
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                }
            }
        };
        if stop_rx.try_recv().is_ok() {
            return;
        }
        let port = listener
            .local_addr()
            .map(|address| address.port())
            .unwrap_or(preferred_port);
        state
            .server_port
            .store(port, std::sync::atomic::Ordering::Relaxed);
        state
            .server_started
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let router = Router::new()
            .route("/", get(index))
            .route("/overlay", get(overlay))
            .route("/editor", get(editor))
            .route("/ws", get(websocket))
            .route("/api/health", get(health))
            .route("/api/style", get(get_style).put(put_style))
            .with_state(state.clone());
        if let Err(error) = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = stop_rx.await;
            })
            .await
        {
            eprintln!("字幕服务已停止：{error}");
        }
        state
            .server_started
            .store(false, std::sync::atomic::Ordering::Relaxed);
    });
}

pub async fn restart(state: &AppState) {
    let stop = state
        .server_stop
        .lock()
        .ok()
        .and_then(|mut current| current.take());
    if let Some(stop) = stop {
        let _ = stop.send(());
    }
    for _ in 0..20 {
        if !state
            .server_started
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    spawn(state.clone());
}

pub fn local_ipv4() -> String {
    let socket = match std::net::UdpSocket::bind(("0.0.0.0", 0)) {
        Ok(socket) => socket,
        Err(_) => return "127.0.0.1".into(),
    };
    if socket.connect(("8.8.8.8", 80)).is_ok() {
        if let Ok(address) = socket.local_addr() {
            if let std::net::IpAddr::V4(ip) = address.ip() {
                return ip.to_string();
            }
        }
    }
    "127.0.0.1".into()
}

async fn index() -> impl IntoResponse {
    axum::response::Redirect::temporary("/editor")
}

async fn overlay() -> Html<&'static str> {
    Html(OVERLAY_HTML)
}

async fn editor() -> Html<&'static str> {
    Html(EDITOR_HTML)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "service": "voice-caption-studio",
        "port": state.server_port.load(std::sync::atomic::Ordering::Relaxed),
    }))
}

async fn get_style(State(state): State<AppState>) -> Json<SubtitleStyle> {
    Json(state.config.read().await.style.clone())
}

async fn put_style(
    State(state): State<AppState>,
    Json(mut style): Json<SubtitleStyle>,
) -> impl IntoResponse {
    style.normalize();
    {
        let mut config = state.config.write().await;
        config.style = style.clone();
    }
    if let Err(error) = state.save_config().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response();
    }
    state.publish_style(style).await;
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

async fn websocket(upgrade: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let init = match serde_json::to_string(&state.init_message().await) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("序列化字幕初始状态失败：{error}");
            return;
        }
    };
    if sender.send(Message::Text(init.into())).await.is_err() {
        return;
    }
    let mut subscription = state.subtitle_tx.subscribe();
    loop {
        tokio::select! {
            message = subscription.recv() => {
                match message {
                    Ok(message) => {
                        if let Ok(text) = serde_json::to_string(&message) {
                            if sender.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

const OVERLAY_HTML: &str = r###"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>声译 OBS Overlay</title>
<style>
:root{--cn-size:34px;--en-size:23px;--cn-color:#fff;--en-color:#a7f3d0;--cn-opacity:1;--en-opacity:.94;--bg:rgba(7,17,31,.68);--outline:2px;--outline-color:#020617;--spacing:1.3}
*{box-sizing:border-box}html,body{width:100%;height:100%;margin:0;background:transparent;overflow:hidden}
#caption{position:absolute;left:50%;top:50%;width:57%;display:none;padding:14px 22px;border-radius:4px;color:#fff;text-align:center;background:var(--bg);line-height:var(--spacing);font-family:"Microsoft YaHei","Segoe UI",sans-serif;transform:translate(-50%,-50%)}
#caption.side{display:flex;align-items:baseline;justify-content:center;gap:22px}
#cn,#en{display:block;will-change:opacity,transform}
#cn{color:var(--cn-color);opacity:var(--cn-opacity);font-size:var(--cn-size);font-weight:600;text-shadow:var(--outline) var(--outline) 0 var(--outline-color),0 3px 12px #0008}
#en{margin-top:6px;color:var(--en-color);opacity:var(--en-opacity);font-size:var(--en-size);font-family:"Segoe UI",Arial,sans-serif;text-shadow:0 2px 8px #0008}
.text-in{animation:caption-text-in .28s cubic-bezier(.22,.8,.28,1) both}@keyframes caption-text-in{from{opacity:.05;transform:translateY(7px);filter:blur(1px)}to{opacity:1;transform:translateY(0);filter:blur(0)}}
.side #en{margin-top:0}.temporary #cn{opacity:.74}
</style></head>
<body><div id="caption"><span id="cn"></span><span id="en"></span></div>
<script>
const box=document.getElementById("caption"),cn=document.getElementById("cn"),en=document.getElementById("en");let style={},last=null,hideTimer=null;
function rgba(hex,a){let h=(hex||"#07111f").replace("#","");if(h.length===3)h=h.split("").map(x=>x+x).join("");let n=parseInt(h,16);return"rgba("+(n>>16&255)+","+(n>>8&255)+","+(n&255)+","+a+")"}
function apply(s){style=s||{};const canvasWidth=Math.max(320,Number(style.canvasWidth)||1920),canvasHeight=Math.max(180,Number(style.canvasHeight)||1080),x=Math.max(0,Math.min(canvasWidth,Number.isFinite(Number(style.subtitleX))?Number(style.subtitleX):canvasWidth/2)),y=Math.max(0,Math.min(canvasHeight,Number.isFinite(Number(style.subtitleY))?Number(style.subtitleY):canvasHeight*.83)),maxWidth=Math.max(200,Number(style.maxWidth)||1100);document.documentElement.style.setProperty("--cn-size",(style.chinese?.fontSize||34)+"px");document.documentElement.style.setProperty("--en-size",(style.english?.fontSize||23)+"px");document.documentElement.style.setProperty("--cn-color",style.chinese?.color||"#fff");document.documentElement.style.setProperty("--en-color",style.english?.color||"#a7f3d0");document.documentElement.style.setProperty("--cn-opacity",style.chinese?.opacity??1);document.documentElement.style.setProperty("--en-opacity",style.english?.opacity??.94);document.documentElement.style.setProperty("--bg",rgba(style.backgroundColor,style.backgroundOpacity??.68));document.documentElement.style.setProperty("--outline",(style.outlineWidth||2)+"px");document.documentElement.style.setProperty("--outline-color",style.outlineColor||"#020617");document.documentElement.style.setProperty("--spacing",style.lineSpacing||1.3);box.style.fontFamily=style.chinese?.fontFamily||"Microsoft YaHei,sans-serif";box.style.left=(x/canvasWidth*100)+"%";box.style.top=(y/canvasHeight*100)+"%";box.style.width=Math.min(96,Math.max(12,maxWidth/canvasWidth*100))+"%";box.style.maxWidth="none";box.classList.toggle("side",style.layout==="sideBySide");box.style.textAlign=style.alignment||"center";render()}
function swapText(element,value){if(element.textContent===value)return false;element.classList.remove("text-in");element.textContent=value;void element.offsetWidth;element.classList.add("text-in");return true}
function render(){if(hideTimer){clearTimeout(hideTimer);hideTimer=null}if(!last){box.style.display="none";return}if(last.kind==="partial"&&!style.showTemporary||last.kind==="final"&&!style.showFinal){box.style.display="none";return}const language=String(last.language||"").toLowerCase(),englishSource=language==="english"||language==="en"||language.startsWith("en-")||language.startsWith("english "),topText=englishSource?(last.english||""):(last.chinese||""),bottomText=englishSource?(last.chinese||""):(last.english||"");swapText(cn,topText);swapText(en,bottomText);en.style.display=bottomText?"":"none";box.classList.toggle("temporary",last.kind==="partial");const visible=Boolean(topText||bottomText);box.style.display=visible?(style.layout==="sideBySide"?"flex":"block"):"none";if(visible){const seconds=Math.max(1,Math.min(300,Number(style.hideAfterSeconds)||10));hideTimer=setTimeout(()=>{box.style.display="none"},seconds*1000)}}
function connect(){const protocol=location.protocol==="https:"?"wss":"ws";const ws=new WebSocket(protocol+"://"+location.host+"/ws");ws.onmessage=e=>{try{const m=JSON.parse(e.data);if(m.type==="init"){apply(m.style);last=m.subtitle;render()}if(m.type==="style"){apply(m.style)}if(m.type==="subtitle"){last=m.event;render()}}catch(err){console.warn(err)}};ws.onclose=()=>setTimeout(connect,1200);ws.onerror=()=>ws.close()}connect();
</script></body></html>"###;

const EDITOR_HTML: &str = r###"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>声译字幕编辑器</title>
<style>
:root{color-scheme:dark;font-family:Segoe UI,"Microsoft YaHei",sans-serif;color:#dbe7f4;background:#0a111c}body{max-width:900px;margin:0 auto;padding:36px 28px}h1{margin:8px 0;color:#eff8ff;font-size:29px}p{color:#7d92a4;font-size:13px}.top{display:flex;align-items:end;justify-content:space-between;margin-bottom:28px}.badge{padding:7px 11px;border:1px solid #1b403d;border-radius:20px;color:#8bdbb4;background:#112927;font-size:11px}.grid{display:grid;grid-template-columns:1fr 1fr;gap:14px}.card{padding:20px;border:1px solid #1b3042;border-radius:12px;background:#101c2a}.card h2{margin:0 0 17px;font-size:15px}.field{display:flex;flex-direction:column;gap:7px;margin-bottom:14px;color:#8398a9;font-size:11px}input,select{height:37px;padding:0 10px;border:1px solid #2b4357;border-radius:7px;outline:none;color:#d2e0ec;background:#0d1b2a}input[type=color]{padding:3px;width:100%}.range{display:flex;align-items:center;gap:10px}.range input{flex:1;accent-color:#78e4b5}.range output{width:37px;color:#9db4c4;font-family:monospace;font-size:11px;text-align:right}.preview{height:230px;position:relative;display:flex;align-items:end;justify-content:center;overflow:hidden;margin-top:20px;border-radius:9px;background:linear-gradient(135deg,#1a2c3a,#0c1725)}.caption{max-width:90%;margin-bottom:25px;padding:12px 18px;text-align:center;background:#07111fb0}.cn{font-size:23px}.en{margin-top:5px;color:#a7f3d0;font-size:16px}.actions{display:flex;justify-content:flex-end;gap:9px;margin-top:17px}button{height:36px;padding:0 14px;border:1px solid #315b50;border-radius:7px;color:#c8f3dc;background:#2a5b50;font-weight:700;cursor:pointer}button.secondary{color:#a6bac8;border-color:#30475a;background:transparent}.saved{margin-right:auto;align-self:center;color:#7cb697;font-size:11px}@media(max-width:700px){.grid{grid-template-columns:1fr}}
</style></head><body><div class="top"><div><div style="color:#6c879d;font:10px monospace;letter-spacing:.15em">BROWSER EDITOR</div><h1>字幕样式编辑器</h1><p>修改会实时同步到已连接的 OBS 浏览器源。</p></div><span class="badge">● 本机服务</span></div>
<div class="grid"><div class="card"><h2>中英文文字</h2><label class="field">中文字号<div class="range"><input id="cnSize" type="range" min="18" max="64"><output></output></div></label><label class="field">中文颜色<input id="cnColor" type="color"></label><label class="field">英文字号<div class="range"><input id="enSize" type="range" min="14" max="48"><output></output></div></label><label class="field">英文颜色<input id="enColor" type="color"></label><label class="field">布局<select id="layout"><option value="stacked">上下布局</option><option value="sideBySide">左右布局</option></select></label></div><div class="card"><h2>画面设置</h2><label class="field">位置<select id="position"><option value="top">顶部</option><option value="center">居中</option><option value="bottom">底部</option></select></label><label class="field">对齐<select id="alignment"><option value="left">左对齐</option><option value="center">居中</option><option value="right">右对齐</select></label><label class="field">背景透明度<div class="range"><input id="bgOpacity" type="range" min="0" max="100"><output></output></div></label><label class="field">无字幕自动隐藏（秒）<input id="hideAfterSeconds" type="number" min="1" max="300"></label><div class="preview"><div class="caption"><div class="cn">欢迎使用声译实时字幕</div><div class="en">Welcome to live caption studio</div></div></div></div></div><div class="actions"><span id="saved" class="saved"></span><button class="secondary" id="reset">恢复默认</button><button id="save">保存样式</button></div>
<script>
const $=id=>document.getElementById(id),fields={cnSize:["chinese","fontSize"],cnColor:["chinese","color"],enSize:["english","fontSize"],enColor:["english","color"],layout:["layout"],position:["position"],alignment:["alignment"],bgOpacity:["backgroundOpacity"],hideAfterSeconds:["hideAfterSeconds"]};let style;
const defaults={chinese:{fontFamily:"Microsoft YaHei, sans-serif",fontSize:34,color:"#ffffff",opacity:1},english:{fontFamily:"Segoe UI, sans-serif",fontSize:23,color:"#a7f3d0",opacity:.94},layout:"stacked",alignment:"center",position:"bottom",canvasWidth:1920,canvasHeight:1080,subtitleX:960,subtitleY:900,maxWidth:1100,lineSpacing:1.3,showTemporary:true,showFinal:true,hideAfterSeconds:10,backgroundColor:"#07111f",backgroundOpacity:.68,outlineWidth:2,outlineColor:"#020617",shadow:true};
function get(obj,path){return path.reduce((v,k)=>v?.[k],obj)}function set(obj,path,val){let x=obj;path.slice(0,-1).forEach(k=>x=x[k]);x[path[path.length-1]]=val}
function render(){Object.entries(fields).forEach(([id,path])=>{let el=$(id),v=get(style,path);if(id==="bgOpacity")v=Math.round(v*100);el.value=v;let o=el.closest(".range")?.querySelector("output");if(o)o.textContent=Math.round(v)+(id==="bgOpacity"?"%":"")});document.querySelector(".caption").style.background="rgba(7,17,31,"+style.backgroundOpacity+")";document.querySelector(".cn").style.fontSize=style.chinese.fontSize+"px";document.querySelector(".cn").style.color=style.chinese.color;document.querySelector(".en").style.fontSize=style.english.fontSize+"px";document.querySelector(".en").style.color=style.english.color}
Object.entries(fields).forEach(([id,path])=>$(id).addEventListener("input",e=>{let v=e.target.value;if(id==="cnSize"||id==="enSize"||id==="hideAfterSeconds")v=Number(v);if(id==="hideAfterSeconds")v=Math.max(1,Math.min(300,Math.round(v||10)));if(id==="bgOpacity")v=Number(v)/100;set(style,path,v);if(id==="position"){style.subtitleX=Math.round((style.canvasWidth||1920)*.5);style.subtitleY=Math.round((style.canvasHeight||1080)*(v==="top"?.16:v==="center"?.5:.83))}render();queueSave()}));
let timer;function queueSave(){clearTimeout(timer);timer=setTimeout(saveStyle,350)}async function saveStyle(){await fetch("/api/style",{method:"PUT",headers:{"content-type":"application/json"},body:JSON.stringify(style)});$("saved").textContent="已同步到 OBS · "+new Date().toLocaleTimeString()}async function load(){try{style=await (await fetch("/api/style")).json()}catch(e){style=defaults}render()}$("save").onclick=saveStyle;$("reset").onclick=()=>{style=JSON.parse(JSON.stringify(defaults));render();saveStyle()};load();
</script></body></html>"###;
