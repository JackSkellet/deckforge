#!/usr/bin/env python3
"""Actual host/native-plugin render and notification tests against a local Unix RPC fixture."""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import time
from PIL import Image
from system_interop import Host
from sample_interop import require

ID='com.elgato.discord'

class Discord:
    def __init__(self,path):
        self.listener=socket.socket(socket.AF_UNIX);self.listener.bind(str(path));self.listener.listen()
        self.peer=None;self.lock=threading.Lock();self.commands=[];self.errors=[];self.done=False
        self.voice={'mute':False,'deaf':False,'mode':{'type':'VOICE_ACTIVITY'},'input':{'volume':50},'output':{'volume':100}}
        self.thread=threading.Thread(target=self.run,daemon=True);self.thread.start()
    def read(self,n):
        data=b''
        while len(data)<n:
            more=self.peer.recv(n-len(data))
            if not more:raise EOFError()
            data+=more
        return data
    def send(self,value):
        data=json.dumps(value).encode()
        with self.lock:self.peer.sendall(struct.pack('<II',1,len(data))+data)
    def run(self):
        try:
            self.peer,_=self.listener.accept()
            while True:
                opcode,size=struct.unpack('<II',self.read(8));assert size<=1024*1024
                value=json.loads(self.read(size))
                if opcode==0:
                    self.send({'cmd':'DISPATCH','evt':'READY','data':{}});continue
                if opcode!=1:continue
                self.commands.append(value)
                cmd=value['cmd'];args=value.get('args',{})
                if cmd=='AUTHENTICATE':
                    assert args['access_token']=='fixture-access'
                    data={'user':{'id':'fixture-user'},'scopes':['rpc','identify','rpc.voice.read','rpc.voice.write','rpc.notifications.read','messages.read']}
                elif cmd=='GET_VOICE_SETTINGS':data=self.voice
                elif cmd=='SET_VOICE_SETTINGS':
                    self.voice.update(args);data=self.voice
                elif cmd=='GET_GUILDS':data={'guilds':[{'id':'fixture-guild','name':'Fixture'}]}
                elif cmd=='GET_CHANNELS':data={'channels':[{'id':'fixture-text','name':'Chat','type':0}]}
                elif cmd in ('GET_CHANNEL','GET_SELECTED_VOICE_CHANNEL'):
                    data={'id':'fixture-voice','guild_id':'fixture-guild','voice_states':[]}
                else:data={}
                self.send({'cmd':cmd,'nonce':value['nonce'],'data':data})
        except (EOFError,ConnectionError,OSError):pass
        except Exception as error:self.errors.append(str(error))
    def wait(self,predicate,description):
        until=time.monotonic()+10
        while time.monotonic()<until:
            assert not self.errors,self.errors
            if predicate():return
            time.sleep(.02)
        raise AssertionError(description)
    def close(self):
        self.done=True
        if self.peer:
            try:self.peer.shutdown(socket.SHUT_RDWR)
            except OSError:pass
            self.peer.close()
        self.listener.close();self.thread.join(timeout=2)

def main():
    project=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    parser.add_argument('--package',type=Path,default=project/'target/release/discord-for-deckforge.streamDeckPlugin')
    parser.add_argument('--report',type=Path)
    args=parser.parse_args();binary=args.binary.resolve();package=args.package.resolve()
    report={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'package_sha256':hashlib.sha256(package.read_bytes()).hexdigest(),'boundary':'Real native plugin/host, mocked Discord Unix socket only. No USB, real Discord or external OAuth/widget request.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-discord-render-') as directory:
        temp=Path(directory);runtime=temp/'runtime';runtime.mkdir()
        old={key:os.environ.get(key) for key in ('XDG_RUNTIME_DIR','TMPDIR','TMP','TEMP')}
        for key in old:os.environ[key]=str(runtime)
        server=Discord(runtime/'discord-ipc-0')
        host=None
        try:
            subprocess.run([str(binary),'install',str(package),'--destination',str(temp/'plugins')],check=True,capture_output=True)
            private=temp/'state/plugin-data'/hashlib.sha256(ID.encode()).hexdigest();private.mkdir(parents=True,mode=0o700)
            config=private/'discord-auth.json';config.write_text(json.dumps({'enabled':True,'client_id':'123456789012345678','client_secret':'fixture-secret','redirect_uri':'','scopes':['rpc'],'token':{'access_token':'fixture-access','refresh_token':'fixture-refresh','expires_at':int(time.time())+3600}}));config.chmod(0o600)
            host=Host(binary,[temp/'plugins'/f'{ID}.sdPlugin'],temp/'state',temp/'host.log');host.ready()
            host.wait(lambda s:any(p['id']==ID and p['connected'] for p in s['plugins']),'native registration')
            server.wait(lambda:any(c['cmd']=='GET_VOICE_SETTINGS' for c in server.commands),'stored session did not authenticate')
            def assign(index,suffix,settings):
                host.post({'command':'assign','controller':'Keypad','index':index,'uuid':ID+'.'+suffix})
                slot=next(s for s in host.state()['slots'] if s['controller']=='Keypad' and s['index']==index)
                host.post({'command':'settings','context':slot['context'],'settings':settings})
                return slot['context']
            horizontal=assign(0,'volumecontrolbutton',{'device':'output','mode':'adjust','icon':'dynamic_h'})
            vertical=assign(1,'volumecontrolbutton',{'device':'output','mode':'adjust','icon':'dynamic_v'})
            notice=assign(2,'notifications',{'select-press':'open'})
            stats=assign(3,'serverstats',{'server':'fixture-guild','select-messages':'display','select-users':'active','select-press':'nothing'})
            def pixels(snapshot,context):
                slot=next(s for s in snapshot['slots'] if s['context']==context)
                return Image.open(io.BytesIO(base64.b64decode(slot['preview']))).convert('RGB')
            def geometry(s):
                h=pixels(s,horizontal);v=pixels(s,vertical)
                return h.getpixel((20,81))[1]>180 and h.getpixel((75,81))[1]<160 and v.getpixel((99,85))[1]>180
            host.wait(geometry,'horizontal/vertical volume meter pixels')
            server.send({'cmd':'DISPATCH','evt':'VOICE_SETTINGS_UPDATE','data':{'output':{'volume':150}}})
            changed=host.wait(lambda s:pixels(s,horizontal).getpixel((75,81))[1]>180,'live volume meter update')
            report['horizontal_png_sha256']=hashlib.sha256(base64.b64decode(next(s for s in changed['slots'] if s['context']==horizontal)['preview'])).hexdigest()
            server.wait(lambda:any(c['cmd']=='SUBSCRIBE' and c.get('evt')=='MESSAGE_CREATE' and c.get('args',{}).get('channel_id')=='fixture-text' for c in server.commands),'server message subscription')
            server.send({'cmd':'DISPATCH','evt':'NOTIFICATION_CREATE','data':{'channel_id':'fixture-text','message':{'id':'message-one','author':{'id':'member'}}}})
            server.send({'cmd':'DISPATCH','evt':'MESSAGE_CREATE','data':{'channel_id':'fixture-text','message':{'id':'message-one','author':{'id':'member'}}}})
            def titles(s):return {v['context']:v['title'] for v in s['slots']}
            host.wait(lambda s:titles(s).get(notice)=='1' and '1 messages' in titles(s).get(stats,'') and '1 active' in titles(s).get(stats,''),'notification/server count feedback')
            host.post({'command':'input','event':'keyDown','index':2});host.post({'command':'input','event':'keyUp','index':2})
            server.wait(lambda:any(c['cmd']=='SELECT_TEXT_CHANNEL' and c.get('args',{}).get('channel_id')=='fixture-text' for c in server.commands),'notification open routing')
            host.post({'command':'settings','context':horizontal,'settings':{'device':'output','mode':'adjust','icon':'static'}})
            host.wait(lambda s:pixels(s,horizontal).getpixel((20,81))[1]<180,'static image restored after dynamic mode')
            require(not any(c['cmd']=='AUTHORIZE' for c in server.commands),'Saved authorization opened a new consent prompt')
            require(not server.errors,str(server.errors))
            report['notifications_and_counts']=True
            assign(4,'pushto.mute',{})
            before=len(server.commands)
            host.post({'command':'input','event':'keyDown','index':4})
            server.wait(lambda:any(c['cmd']=='SET_VOICE_SETTINGS' and c.get('args',{}).get('mute') is True for c in server.commands[before:]),'held mute activation')
            host.stop();require(host.process.returncode==0,'Host shutdown failed');host=None
            server.wait(lambda:any(c['cmd']=='SET_VOICE_SETTINGS' and c.get('args',{}).get('mute') is False for c in server.commands[before:]),'held mute release before native child exit')
            report['held_control_released_on_host_shutdown']=True
            report['rpc_commands']=sorted(set(c['cmd'] for c in server.commands))
        finally:
            if host:host.stop()
            server.close()
            for key,value in old.items():
                if value is None:os.environ.pop(key,None)
                else:os.environ[key]=value
    if args.report:args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: real host/native-plugin rendered horizontal/vertical/live volume and static reset; notification routing and guild counts; stored authorization resumed without consent prompt. Mock Discord only.')

if __name__=='__main__':main()
