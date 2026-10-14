#!/usr/bin/env python3
"""Actual host export/import/restart preserves explicit and legacy key mapping modes."""
import argparse,hashlib,json,tempfile,zipfile
from pathlib import Path
from system_interop import Host

def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/logical-hotkeys/settings.json'));a=p.parse_args();a.output.parent.mkdir(parents=True,exist_ok=True)
 with tempfile.TemporaryDirectory(prefix='deckforge-logical-settings-') as d:
  root=Path(d);state=root/'state';host=Host(a.binary.resolve(),[],state,root/'host.log').ready()
  chord={'QTKeyCode':89,'KeyModifiers':0x10202,'Future':9}
  cases=[('com.elgato.streamdeck.system.hotkey',{'Hotkeys':[chord],'Future':1}),('com.elgato.streamdeck.system.hotkey',{'Hotkeys':[{**chord,'linuxKeyMode':'logical'}],'Future':2}),('com.elgato.streamdeck.system.hotkeyswitch',{'Hotkeys':[{**chord,'linuxKeyMode':'logical'},{**chord,'linuxKeyMode':'physical'}]}),('com.deckforge.system.clipboard',{'operation':'paste','terminal_shortcuts':True,'linuxKeyMode':'logical'}),('com.elgato.streamdeck.system.text',{'pastedText':'preserved','isTypingMode':False,'Hotkey':{**chord,'linuxKeyMode':'logical'}}),('com.elgato.streamdeck.system.hotkey',{'Hotkeys':[{**chord,'linuxKeyMode':'future'}]})]
  cases.extend([('com.elgato.streamdeck.system.text',{'pastedText':'legacy paste','isTypingMode':False,'Hotkey':{'QTKeyCode':86,'KeyModifiers':8192,'KeyCtrl':True,'KeyShift':True,'Future':9}}),('com.elgato.streamdeck.system.hotkey',{'Hotkeys':[{'QTKeyCode':65,'KeyModifiers':8192,'KeyCtrl':True,'Future':8}]})])
  def verify():
   slots={s['index']:s for s in host.state()['slots'] if s['controller']=='Keypad'}
   for i,(uuid,settings) in enumerate(cases):assert slots[i]['uuid']==uuid and slots[i]['settings']==settings,(i,slots[i])
  try:
   for i,(uuid,settings) in enumerate(cases):
    host.post({'command':'assign','controller':'Keypad','index':i,'uuid':uuid})
    slot=next(s for s in host.state()['slots'] if s['controller']=='Keypad' and s['index']==i)
    if uuid in ['com.elgato.streamdeck.system.text','com.deckforge.system.clipboard']:assert slot['settings']['linuxKeyMode']=='logical','Fresh paste action did not follow desktop layout'
    host.post({'command':'settings','context':slot['context'],'settings':settings})
   verify();before=(state/'session.json').read_bytes();export=root/'logical.streamDeckProfile';host.post({'command':'export_profile','path':str(export)})
   assert (state/'session.json').read_bytes()==before,'Export rewrote stored modes'
   host.post({'command':'import_profile','path':str(export)});verify()
   host.stop();host=Host(a.binary.resolve(),[],state,root/'restart.log').ready();verify()
   assert host.state()['typing_status']['phase']=='disabled'
  finally:host.stop()
 a.output.write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'legacy_missing_mode_preserved':True,'logical_physical_and_unknown_modes_preserved':True,'custom_text_and_terminal_shortcut_modes':True,'export_import_restart':True,'actions_executed':False},indent=2)+'\n')
 print('PASS: actual host preserves legacy, logical, physical, unknown modes and opaque settings through export/import/restart. No actions executed.')
if __name__=='__main__':main()
