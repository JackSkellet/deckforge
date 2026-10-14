// Independent Linux implementation. Node 24 supplies the WebSocket client.
import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';

const args = Object.fromEntries(Array.from({length: (process.argv.length - 2) / 2}, (_, i) =>
  [process.argv[2 + i * 2], process.argv[3 + i * 2]]));
const socket = new WebSocket(`ws://127.0.0.1:${args['-port']}`);
const actions = new Map(), inspectors = new Set(), pending = new Map();
let helper, stopping = false, retry, nextId = 0, radio = 'unavailable', devices = [], error = '';
const send = (event, context, payload = {}) => {
  if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({event, context, payload}));
};
const escape = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&apos;'}[c]));
const image = svg => `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`;
const short = (s, max) => [...String(s)].slice(0, max).join('');
function choices(action) {
  const address = action.kind === 'battery-dial' ? action.settings.pinnedAddress : action.settings.address;
  return address ? devices.filter(d => d.address.toLowerCase() === address.toLowerCase()) : devices;
}
function current(action) {
  if (action.kind === 'device' && !action.settings.address) return undefined;
  const list = choices(action);
  return list[((action.index || 0) % list.length + list.length) % list.length];
}
function paint(context, action) {
  const device = current(action), connected = device?.connected;
  const dial = action.kind === 'battery-dial', width = dial ? 200 : 144, height = dial ? 100 : 144;
  let title = action.settings.customLabel?.trim() || device?.name || action.settings.deviceName || 'Choose device';
  let status = connected ? 'Connected' : 'Disconnected';
  if (action.kind === 'radio') { title = 'Bluetooth'; status = radio === 'on' ? 'On' : radio === 'off' ? 'Off' : 'Unavailable'; }
  if (action.kind === 'disconnect-all') { title = 'Disconnect all'; status = `${devices.filter(d => d.connected).length} connected`; }
  if (radio !== 'on' && action.kind !== 'radio') status = radio === 'off' ? 'Bluetooth off' : 'Unavailable';
  if (action.busy) status = 'Working…';
  if (action.failure) status = 'Failed: see settings';
  const battery = device?.battery;
  const batteryLabel = Number.isFinite(battery) ? `${battery}%` : 'Battery unavailable';
  const color = radio === 'on' && (connected || action.kind === 'radio') ? '#4bd3b0' : '#a4b3bd';
  const label = action.settings.showLabel === false && !dial ? '' : escape(short(title, dial ? 24 : 17));
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="100%" height="100%" fill="#202d35"/><path d="M18 15l22 26-11 11V4l11 11-22 26" transform="translate(${dial ? 1 : 44} ${dial ? 4 : 10})" fill="none" stroke="${color}" stroke-width="3" stroke-linejoin="round"/><g fill="#edf2f5" font-family="sans-serif" text-anchor="middle"><text x="${width/2}" y="${dial ? 63 : 91}" font-size="${dial ? 14 : 13}">${label}</text><text x="${width/2}" y="${dial ? 82 : 112}" font-size="11" fill="${color}">${escape(status)}</text>${dial || (action.settings.showBattery !== false && Number.isFinite(battery)) ? `<text x="${dial ? 125 : 72}" y="${dial ? 29 : 132}" font-size="${dial ? 17 : 12}">${escape(batteryLabel)}</text>` : ''}</g></svg>`;
  if (dial) send('setFeedback', context, {canvas:image(svg)});
  else { send('setTitle', context, {title:'',target:0}); send('setImage',context,{image:image(svg),target:0}); }
}
function updateInspectors() {
  for (const context of inspectors) send('sendToPropertyInspector', context,
    {action:'devices', radio, devices, helperUp:radio !== 'unavailable', error:actions.get(context)?.failure || error});
}
function redraw() { for (const [context, action] of actions) paint(context, action); updateInspectors(); }
function write(command) {
  if (!helper?.stdin.writable) throw new Error('Bluetooth service is unavailable');
  helper.stdin.write(`${JSON.stringify(command)}\n`);
}
function request(command) {
  const reqId = ++nextId;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {pending.delete(reqId); reject(new Error('Bluetooth request timed out'));}, 35000);
    pending.set(reqId, {resolve,reject,timer});
    try { write({...command,reqId}); } catch (e) { clearTimeout(timer); pending.delete(reqId); reject(e); }
  });
}
function startHelper() {
  const child = helper = spawn(fileURLToPath(new URL('./fk-bt-helper.py', import.meta.url)), [], {stdio:['pipe','pipe','pipe']});
  child.stdin.on('error', () => {});
  child.stderr.on('data', data => console.error(String(data).trim()));
  createInterface({input:child.stdout}).on('line', line => {
    try {
      const message = JSON.parse(line);
      if (message.type === 'radio') { radio = message.state; if (radio !== 'unavailable') error = ''; }
      else if (message.type === 'snapshot') devices = message.devices;
      else if (message.type === 'result') {
        const waiter = pending.get(message.reqId);
        if (waiter) { clearTimeout(waiter.timer); pending.delete(message.reqId);
          message.ok ? waiter.resolve() : waiter.reject(new Error(message.error || 'Bluetooth command failed')); }
      } else if (message.type === 'log') { console.error(message.message); if (message.level === 'error') error = message.message; }
      redraw();
    } catch (e) { console.error('Invalid helper response:', e.message); }
  });
  let ended = false;
  const unavailable = reason => {
    if (ended) return; ended = true; helper = undefined;
    radio = 'unavailable'; devices = []; error = reason; console.error(reason);
    for (const item of pending.values()) {clearTimeout(item.timer); item.reject(new Error(reason));} pending.clear();
    redraw(); if (!stopping) retry = setTimeout(startHelper, 3000);
  };
  child.on('error', e => unavailable(e.message));
  child.on('exit', code => unavailable(`Bluetooth helper stopped (${code})`));
}
async function activate(context, action) {
  if (action.busy) return;
  action.busy = true; action.failure = ''; paint(context, action);
  try {
    if (action.kind === 'radio') await request({cmd:'radio',state:'toggle'});
    else if (action.kind === 'disconnect-all') await request({cmd:'disconnectAll'});
    else {
      const device = current(action);
      if (!device) throw new Error('Choose a paired device in settings first');
      await request({cmd:device.connected ? 'disconnect' : 'connect',id:device.id});
    }
  } catch (e) { action.failure = e.message; console.error(e.message); send('showAlert',context); }
  finally {action.busy = false; redraw();}
}
socket.addEventListener('open', () => {
  socket.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']}));
  startHelper();
});
socket.addEventListener('message', event => {
  try {
    const message = JSON.parse(event.data), {context,payload = {}} = message;
    if (message.event === 'willAppear') {
      const action = {kind:message.action.split('.').at(-1),settings:payload.settings || {},index:0};
      actions.set(context,action);
      if (action.kind === 'battery-dial') send('setFeedbackLayout',context,{layout:'layouts/battery.json'});
      paint(context,action);
    } else if (message.event === 'willDisappear') {actions.delete(context);inspectors.delete(context);}
    else if (message.event === 'didReceiveSettings' && actions.has(context)) {
      actions.get(context).settings = payload.settings || {}; actions.get(context).failure = ''; redraw();
    } else if (message.event === 'propertyInspectorDidAppear') {inspectors.add(context); updateInspectors();}
    else if (message.event === 'propertyInspectorDidDisappear') inspectors.delete(context);
    else if (message.event === 'sendToPlugin') {
      inspectors.add(context);
      if (payload.action === 'refresh-devices') {if (actions.has(context)) actions.get(context).failure = '';write({cmd:'refresh'});}
      updateInspectors();
    } else if (actions.has(context)) {
      const action = actions.get(context);
      if (message.event === 'keyDown' || message.event === 'dialDown') void activate(context,action);
      else if (message.event === 'dialRotate' || message.event === 'touchTap') {
        action.index += message.event === 'touchTap' ? 1 : Number(payload.ticks || 0); paint(context,action);
      }
    }
  } catch (e) {console.error(e.message);}
});
function stop() {
  if (stopping) return; stopping = true; clearTimeout(retry);
  for (const item of pending.values()) clearTimeout(item.timer);
  helper?.stdin.end(); helper?.kill(); socket.close();
  setTimeout(() => process.exit(0), 100).unref();
}
socket.addEventListener('close',stop);
socket.addEventListener('error',e => {console.error('Host connection failed',e.message || '');stop();});
process.on('SIGINT',stop); process.on('SIGTERM',stop);
