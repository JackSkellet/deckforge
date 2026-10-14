// Exercise the real native WebKit inspector with an owned protocol server.
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
const {spawn} = require('node:child_process');
const {WebSocketServer} = require('../frontend/node_modules/ws');
const root = path.resolve(__dirname,'..');
const binary = process.argv[2] || path.join(root,'target/debug/deckforge');
const temp = fs.mkdtempSync(path.join(os.tmpdir(),'bluetooth-native-pi-'));
const wss = new WebSocketServer({host:'127.0.0.1',port:0});
const roster = {action:'devices',radio:'on',devices:[{address:'aa:bb',name:'Test headphones',connected:true},{address:'cc:dd',name:'Test mouse',connected:false}]};
let receive, child;
wss.on('connection', ws => ws.on('message', raw => {
  const m = JSON.parse(raw);
  if (m.event === 'sendToPlugin') ws.send(JSON.stringify({event:'sendToPropertyInspector',payload:m.payload.action === 'refresh-devices' ? {...roster,error:'Fixture unavailable'} : roster}));
  if (m.event === 'setSettings') ws.send(JSON.stringify({event:'didReceiveSettings',payload:{settings:m.payload}}));
  if (m.event === 'testResult') receive(m.payload);
}));
(async()=>{
  await new Promise(r=>wss.once('listening',r));
  for(const kind of ['device','battery-dial','radio','disconnect-all']) {
    const dir = path.join(temp,kind);fs.cpSync(path.join(root,'plugins/bluetooth-compat/ui'),dir,{recursive:true});
    fs.appendFileSync(path.join(dir,'inspector.js'), `
const originalConnect = window.connectElgatoStreamDeckSocket;
window.connectElgatoStreamDeckSocket = (...args) => {
 originalConnect(...args);
 const pause = ms => new Promise(r=>setTimeout(r,ms));
 const until = async test => {for(let i=0;i<100;i++){if(test())return;await pause(30);}throw Error('Inspector timeout');};
 (async()=>{
 await until(()=>el('status').textContent.includes('3') || el('status').textContent.includes('2 paired'));
 if(['device','battery-dial'].includes(kind)) {
   if(el('device-fields').hidden) throw Error('Missing device picker');
   el('device').value='cc:dd';el('device').dispatchEvent(new Event('change'));
   await until(()=>settings[kind==='device'?'address':'pinnedAddress']==='cc:dd');
   if(!settings.future.retained)throw Error('Unknown setting lost');
   el('label').value='My mouse';el('label').dispatchEvent(new Event('change'));
   await until(()=>settings.customLabel==='My mouse');
   if(settings.deviceName!=='Test mouse')throw Error('Device name not saved');
 } else if(!el('device-fields').hidden)throw Error('Unexpected picker');
 el('refresh').click();await until(()=>!el('error').hidden);
 if(el('error').textContent!=='Fixture unavailable')throw Error('Error not visible');
 const fields=[...document.querySelectorAll('select,input[type=text],button')].filter(e=>e.getClientRects().length);
 if(document.documentElement.scrollWidth>innerWidth+1 || fields.some(e=>{const r=e.getBoundingClientRect();return r.width<=0||r.left<0||r.right>innerWidth+1}))throw Error('Control overflow');
 socket.send(JSON.stringify({event:'testResult',payload:{ok:true,kind,settings,viewport:[innerWidth,innerHeight]}}));
 })().catch(e=>socket.send(JSON.stringify({event:'testResult',payload:{ok:false,error:String(e)}})));
};`);
    const proof = new Promise((resolve,reject)=>{receive=resolve;setTimeout(()=>reject(Error('Native inspector did not report')),10000).unref();});
    child = spawn(binary,['webview','--visible','--path',path.join(dir,'index.html'),'--port',String(wss.address().port),'--uuid','fixture-context','--register-event','registerPropertyInspector','--info','{}','--action-info',JSON.stringify({action:'com.flowkits.bluetooth.'+kind,payload:{settings:{future:{retained:true}}}})],{stdio:['ignore','ignore','pipe']});
    child.stderr.on('data',()=>{});
    try {const result=await proof;if(!result.ok)throw Error(JSON.stringify(result));console.log('PASS native inspector:',kind,result.viewport);}finally{const done=new Promise(r=>child.once('exit',r));child.kill();await done;child=null;}
  }
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>{child?.kill();for(const client of wss.clients)client.terminate();wss.close();fs.rmSync(temp,{recursive:true,force:true});});
