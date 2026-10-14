let socket, context, kind, settings = {}, devices = [];
const el = id => document.getElementById(id);
function send(event,payload = {}) {if(socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({event,context,payload}));}
function render() {
  const dial = kind === 'battery-dial', field = dial ? 'pinnedAddress' : 'address';
  const selected = settings[field] || '';
  const options = [new Option(dial ? 'All devices · rotate to choose' : 'Choose a device', '')];
  for(const device of devices) options.push(new Option(`${device.name}${device.connected ? ' · Connected' : ''}`,device.address));
  if(selected && !devices.some(d=>d.address === selected)) options.push(new Option(settings.deviceName || selected,selected));
  el('device').replaceChildren(...options); el('device').value = selected;
  el('label').value = settings.customLabel || '';
  el('show-label').checked = settings.showLabel !== false;
  el('show-battery').checked = settings.showBattery !== false;
  el('device-fields').hidden = !['device','battery-dial'].includes(kind);
  el('help').textContent = ({device:'Press the key to connect or disconnect the selected device.',radio:'Press the key to turn Bluetooth on or off. Turning it off disconnects your devices.','disconnect-all':'Press the key to disconnect all connected Bluetooth devices.','battery-dial':'Rotate or tap to select a device. Press the dial to connect or disconnect it.'})[kind] || '';
}
function save(change) {settings = {...settings,...change};send('setSettings',settings);}
window.connectElgatoStreamDeckSocket = (port,uuid,event,info,actionInfo) => {
  context = uuid; const action = JSON.parse(actionInfo); kind = action.action.split('.').at(-1); settings = action.payload?.settings || {};render();
  socket = new WebSocket(`ws://127.0.0.1:${port}`);
  socket.onopen = () => {socket.send(JSON.stringify({event,uuid}));send('sendToPlugin',{action:'get-devices'});};
  socket.onclose = () => {el('status').textContent = 'Plugin disconnected. Reopen these settings after restarting the plugin.';};
  socket.onmessage = event => {
    const message = JSON.parse(event.data), payload = message.payload || {};
    if(message.event === 'didReceiveSettings') {settings = payload.settings || {};render();}
    if(message.event === 'sendToPropertyInspector' && payload.action === 'devices') {
      devices = payload.devices || [];
      const selected = el('device').value;
      const options = [new Option(kind === 'battery-dial' ? 'All devices · rotate to choose' : 'Choose a device','')];
      for(const d of devices) options.push(new Option(`${d.name}${d.connected ? ' · Connected' : ''}`,d.address));
      if(selected && !devices.some(d=>d.address === selected)) options.push(new Option(settings.deviceName || selected,selected));
      el('device').replaceChildren(...options); el('device').value = selected;
      el('status').textContent = payload.radio === 'on' ? `Bluetooth on · ${devices.length} paired devices` : payload.radio === 'off' ? 'Bluetooth is off' : 'Bluetooth service is unavailable';
      el('error').textContent = payload.error || '';el('error').hidden = !payload.error;
    }
  };
};
el('device').onchange = () => {const device = devices.find(d=>d.address === el('device').value);save({[kind === 'battery-dial' ? 'pinnedAddress':'address']:el('device').value,deviceName:device?.name || ''});};
el('label').onchange = () => save({customLabel:el('label').value});
el('show-label').onchange = () => save({showLabel:el('show-label').checked});
el('show-battery').onchange = () => save({showBattery:el('show-battery').checked});
el('refresh').onclick = () => send('sendToPlugin',{action:'refresh-devices'});
