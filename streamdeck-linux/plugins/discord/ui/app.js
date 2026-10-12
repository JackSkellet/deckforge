'use strict';
// Original inspector. Settings retain unknown imported fields; auth never enters them.
let socket, uuid, action, settings = {}, definitions = {}, state = {}, auth = {};
const $ = id => document.getElementById(id);
const names = {server:'Server ID',serverName:'Server name',channel:'Channel ID',guild_id:'Sound server ID',sound_id:'Sound ID',sound_name:'Sound name',emoji_path:'Sound emoji path',input:'Input device ID',output:'Output device ID',inputname:'Input name',outputname:'Output name',user:'User ID',usernick:'User display name',user_mode:'Choose user',volValue:'Percent per dial tick',volume:'Percent per dial tick',volAdjustValue:'Volume adjustment',volSetValue:'Target volume (%)',muteType:'Mute operation',icon:'Icon style',device:'Audio device','select-press':'On press','select-longpress':'On long press','select-messages':'Message count','select-users':'User count'};
function send(event,payload) { if(socket?.readyState===WebSocket.OPEN)socket.send(JSON.stringify({event,context:uuid,action,payload})); }
function plugin(payload){send('sendToPlugin',payload);}
window.connectElgatoStreamDeckSocket = (port,id,registration,info,actionInfo) => {
 uuid=id;const parsed=JSON.parse(actionInfo);action=parsed.action;settings=parsed.payload?.settings??{};
 socket=new WebSocket(`ws://127.0.0.1:${port}`);
 socket.onopen=()=>{socket.send(JSON.stringify({event:registration,uuid}));plugin({command:'status'});};
 socket.onclose=()=>{$('status').textContent='Deckforge connection closed.';};
 socket.onmessage=event=>{const message=JSON.parse(event.data);if(message.event==='didReceiveSettings'){settings=message.payload?.settings??{};renderFields();}if(message.event==='sendToPropertyInspector'){Object.assign(state,message.payload??{});if(message.payload?.auth)Object.assign(auth,message.payload.auth);renderState();refreshLists();}};
 fetch('fields.json').then(r=>r.json()).then(data=>{definitions=data;renderFields();}).catch(()=>{$('error').textContent='Could not load action setting definitions.';});
};
function renderState(){
 $('status').textContent=auth.message||(auth.authenticated?'Discord authorized.':state.unauthorized?'Connect an authorized Discord application.':'Waiting for Discord status.');
 $('error').textContent=state.error||((action?.endsWith('.notifications')||action?.endsWith('.serverstats'))?'Notification history and server statistics are not implemented yet. Imported settings remain intact.':'');
 $('connect-saved').disabled=auth.configured===false;
}
function renderFields(){
 const definition=definitions[action];if(!definition)return;$('title').textContent=definition.name;
 const fields=$('fields');fields.replaceChildren();
 for(const [key,schema] of Object.entries(definition.fields)){
  const row=document.createElement('div');row.dataset.field=key;
  const label=document.createElement('label');label.htmlFor=`setting-${key}`;label.textContent=names[key]||key;row.append(label);
  const input=document.createElement(schema.enum?'select':'input');input.id=`setting-${key}`;input.dataset.key=key;
  if(schema.enum){for(const item of schema.enum){const o=document.createElement('option');o.value=item;o.textContent=item.replaceAll('_',' ').replaceAll('-',' ');input.append(o);}}
  else if(schema.type==='integer'){input.type='number';input.min=schema.minimum;input.max=schema.maximum;input.step='1';}
  else{input.type='text';input.autocomplete='off';input.spellcheck=false;input.setAttribute('list',`list-${key}`);const list=document.createElement('datalist');list.id=`list-${key}`;row.append(list);}
  input.value=settings[key]??schema.default??'';
  input.addEventListener('change',()=>{let value=input.value;if(schema.type==='integer'){value=Number(value);if(!Number.isInteger(value)||value<schema.minimum||value>schema.maximum){$('save-status').textContent=`Enter a whole number from ${schema.minimum} to ${schema.maximum}.`;return;}}settings={...settings,[key]:value};send('setSettings',settings);$('save-status').textContent='Settings saved.';visibility();});row.append(input);
  if(key==='icon'){input.disabled=true;const note=document.createElement('p');note.className='fieldnote';note.textContent='Imported style is preserved. Native dynamic meters and avatars are not implemented yet.';row.append(note);}
  if(key==='volAdjustValue'){const note=document.createElement('p');note.className='fieldnote';note.textContent='Original profile encoding: −25…−1 lowers volume; 0…24 raises it by 1…25%.';row.append(note);}
  fields.append(row);
 }
 if(!Object.keys(definition.fields).length){const p=document.createElement('p');p.className='empty';p.textContent='This action has no additional settings.';fields.append(p);}
 visibility();refreshLists();renderState();
}
function visibility(){const mode=settings.mode??definitions[action]?.fields.mode?.default;for(const row of $('fields').children){const key=row.dataset.field;let show=true;if(['input','inputname'].includes(key)&&mode==='output')show=false;if(['output','outputname'].includes(key)&&mode==='input')show=false;if(['user','usernick'].includes(key)&&action?.endsWith('uservolumecontroldial')&&(settings.user_mode??'any')==='any')show=false;if(key==='volAdjustValue'&&mode!=='adjust')show=false;if(key==='volSetValue'&&mode!=='set')show=false;if(key==='muteType'&&mode!=='mute')show=false;row.hidden=!show;}}
function refreshLists(){for(const [key,source]of Object.entries({server:'servers',guild_id:'servers',channel:'channels',user:'users',input:'inputs',output:'outputs'})){const list=$(`list-${key}`);if(!list)continue;list.replaceChildren();let items=state[source];if(items&&!Array.isArray(items))items=Object.entries(items).map(([id,v])=>typeof v==='string'?{id,name:v}:{id,...v});for(const item of items??[]){if(!item?.id)continue;const option=document.createElement('option');option.value=item.id;option.label=item.name||item.id;list.append(option);}}}
$('connect-saved').onclick=()=>plugin({command:'connect'});
$('connect-new').onclick=()=>{const client_id=$('client-id').value.trim(),client_secret=$('client-secret').value;if(!client_id||!client_secret){$('error').textContent='Enter the client ID and client secret.';return;}const payload={command:'connect',client_id,client_secret,scopes:$('scopes').value.split(/[\s,]+/).filter(Boolean)};if($('redirect').value.trim())payload.redirect_uri=$('redirect').value.trim();plugin(payload);$('client-secret').value='';};
$('disconnect').onclick=()=>plugin({command:'disconnect'});
$('forget').onclick=()=>{if(window.confirm('Forget the application credentials and authorization saved privately by this plugin?'))plugin({command:'forget'});};
