#!/usr/bin/env python3
"""Managed update review/apply/rollback with inert fixtures and simulated devices."""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import stat
import subprocess
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen
import zipfile

from multi_device_interop import assign, inspect, post, press, rows, wait
from plugin_enable_interop import ACTION, PLUGIN, identities, plugin, probe, reject_old_registration
from sample_interop import COUNTER, COUNTER_LOCK_SHA256, require
from system_interop import Host

LEGACY=PLUGIN+'.legacy'
ADDED=PLUGIN+'.added'


def make_package(root, original, version, actions, identifier=PLUGIN, broken=False):
    manifest=json.loads((original/'manifest.json').read_text())
    manifest.update(UUID=identifier,Version=version)
    manifest['Actions']=[{'UUID':uuid,'Name':uuid.rsplit('.',1)[-1],'Controllers':['Keypad'],'States':[{}]} for uuid in actions]
    source=(original/'plugin.mjs').read_text().replace('{kind,pid:process.pid,...extra}', '{kind,pid:process.pid,version:'+json.dumps(version)+',...extra}')
    files={'manifest.json':None,'plugin.mjs':source.encode()}
    if broken:
        architecture={'x86_64':(2,62),'aarch64':(2,183),'i686':(1,3),'armv7l':(1,40)}[platform.machine()]
        header=bytearray(64);header[:7]=b'\x7fELF'+bytes([architecture[0],1,1]);header[16:18]=(2).to_bytes(2,'little');header[18:20]=architecture[1].to_bytes(2,'little')
        files['broken']=bytes(header);manifest['CodePath']='broken'
    files['manifest.json']=json.dumps(manifest).encode()
    path=root/(identifier+'-'+version+'.streamDeckPlugin')
    with zipfile.ZipFile(path,'w',zipfile.ZIP_DEFLATED) as archive:
        for name,content in files.items():
            info=zipfile.ZipInfo(identifier+'.sdPlugin/'+name);info.create_system=3
            info.external_attr=(stat.S_IFREG|(0o755 if name=='broken' else 0o644))<<16
            archive.writestr(info,content)
    return path


def prepare(host, path=None, kind='update', expected=200, selection=None, **overrides):
    current=selection if selection is not None else plugin(host)
    body={'id':current['id'],'expected_path':current['path'],'expected_version':current['version'],'kind':kind}
    if path is not None:body['path']=str(path)
    body.update(overrides)
    request=Request(host.base+'/api/plugin-update/prepare',json.dumps(body).encode(),headers={'Content-Type':'application/json','X-Deckforge-Token':host.token},method='POST')
    try:
        with urlopen(request,timeout=10) as response:status,data=response.status,json.load(response)
    except HTTPError as error:status,data=error.code,json.loads(error.read())
    require(status==expected,f'Prepare expected{expected},got{status}: {data}')
    return data


def apply(host,review,expected=200):
    host.post({'command':'plugin_update_apply','ticket':review['ticket'],'confirm':True},expected=expected)


def version(host,wanted,enabled=True):
    return host.wait(lambda s:any(p['id']==PLUGIN and p['version']==wanted and p['enabled'] is enabled and (p['connected'] if enabled else p['pid'] is None and p['status']=='disabled') for p in s['plugins']),'plugin version '+wanted)


def manifest_version(target):
    return json.loads((target/'manifest.json').read_text())['Version']


def main():
    project=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/debug/deckforge')
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args();binary=args.binary.resolve(strict=True)
    sample=project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'Official dependencies changed')
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-update-') as d:
        root=Path(d);state=root/'state';original,events=probe(root,sample)
        v1=make_package(root,original,'1.0.0',[ACTION,LEGACY]);v2=make_package(root,original,'2.0.0',[ACTION,ADDED])
        wrong=make_package(root,original,'9.0.0',['com.example.wrong.key'],'com.example.wrong')
        broken=make_package(root,original,'3.0.0',[ACTION],broken=True)
        subprocess.run([str(binary),'install',str(v1),'--destination',str(state/'plugins')],check=True,capture_output=True,text=True)
        target=state/'plugins'/(PLUGIN+'.sdPlugin')
        host=Host(binary,[target,sample/'com.elgato.counter.sdPlugin'],state,root/'host.log')
        try:
            host.ready();host.wait(lambda s:len(s['plugins'])==2 and all(p['connected'] for p in s['plugins']),'plugins registered')
            a=host.state()['device'];b='update-mini'
            first=assign(host,a,0,ACTION,{'keep':'first'});legacy=assign(host,a,1,LEGACY,{'keep':'legacy'})
            counter=assign(host,a,2,COUNTER,{'count':5,'incrementBy':1})
            host.post({'command':'device_add','device':b,'model':'mini'})
            second=assign(host,b,0,ACTION,{'keep':'second'})
            host.post({'command':'device_select','device':b})
            before=identities(host,[a,b]);old=plugin(host);counter_pid=plugin(host,'com.elgato.counter')['pid']
            prepare(host,wrong,expected=400);prepare(host,v2,expected=400,expected_version='stale')
            require(plugin(host)['pid']==old['pid'] and manifest_version(target)=='1.0.0','Failed preflight stopped/replaced active version')
            cancelled=prepare(host,v2)
            require(cancelled['comparison']=='upgrade' and cancelled['removed_actions']==[LEGACY] and cancelled['added_actions']==[ADDED],'Review omitted action/version differences')
            require(plugin(host)['pid']==old['pid'] and manifest_version(target)=='1.0.0','Prepare changed active plugin')
            host.post({'command':'plugin_update_cancel','ticket':cancelled['ticket']});apply(host,cancelled,expected=400)
            require(plugin(host)['pid']==old['pid'],'Cancelled review disrupted runtime')
            stale=prepare(host,v2);host.post({'command':'restart_plugin','id':PLUGIN});version(host,'1.0.0')
            restarted_pid=plugin(host)['pid'];apply(host,stale,expected=400)
            require(plugin(host)['pid']==restarted_pid and manifest_version(target)=='1.0.0','Stale ticket stopped current launch')
            old_registration=next(r['registration'] for r in rows(events) if r['kind']=='registered' and r['pid']==restarted_pid)
            update=prepare(host,v2)
            host.post({'command':'plugin_update_apply','ticket':update['ticket'],'confirm':False},expected=400)
            require(plugin(host)['pid']==restarted_pid,'Unconfirmed update changed runtime')
            post(host,a,'input',event='keyDown',index=0)
            wait(host,lambda:any(r['kind']=='keyDown' and r['pid']==restarted_pid and r.get('context')==first for r in rows(events)),'held target input')
            apply(host,update);version(host,'2.0.0')
            v2_pid=plugin(host)['pid']
            wait(host,lambda:any(r['kind']=='registered' and r['pid']==v2_pid and r.get('version')=='2.0.0' for r in rows(events)),'updated executable reports v2')
            require(v2_pid!=restarted_pid and not Path(f'/proc/{restarted_pid}').exists(),'Old version process survived update')
            require(plugin(host,'com.elgato.counter')['pid']==counter_pid,'Update restarted Counter')
            require(identities(host,[a,b])==before,'Update changed retained assignment settings/contexts')
            require(host.state()['device']==b,'Update changed device selection')
            require(next(s for s in inspect(host,a)['slots'] if s['context']==legacy)['available'] is False,'Removed action is not marked unavailable')
            require(plugin(host)['rollback_available'] and plugin(host)['rollback_version']=='1.0.0','Previous version not offered')
            post(host,a,'input',event='keyUp',index=0);reject_old_registration(host,sample,old_registration)
            require(not any(r['pid']==v2_pid and r['kind']=='keyUp' for r in rows(events)),'Old held release reached new runtime')
            press(host,a,2);wait(host,lambda:next(s for s in inspect(host,a)['slots'] if s['context']==counter)['settings']['count']==6,'Counter remains operational')
            rollback=prepare(host,kind='rollback');require(rollback['comparison']=='downgrade','Rollback review missing downgrade label')
            apply(host,rollback);version(host,'1.0.0')
            restored_pid=plugin(host)['pid']
            wait(host,lambda:any(r['kind']=='registered' and r['pid']==restored_pid and r.get('version')=='1.0.0' for r in rows(events)),'restored executable reports v1')
            require(next(s for s in inspect(host,a)['slots'] if s['context']==legacy)['available'] is True,'Rollback did not restore removed action')
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':False,'expected_enabled':True})
            offset=len(rows(events));disabled=prepare(host,v2);require(disabled['was_enabled'] is False,'Disabled review misreported runtime state')
            apply(host,disabled);version(host,'2.0.0',False)
            require(len(rows(events))==offset,'Updating disabled plugin launched it')
            apply(host,prepare(host,kind='rollback'));version(host,'1.0.0',False)
            require(len(rows(events))==offset,'Restoring disabled plugin launched it')
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':True,'expected_enabled':False});version(host,'1.0.0')
            bad=prepare(host,broken);apply(host,bad,expected=400);version(host,'1.0.0')
            require(manifest_version(target)=='1.0.0','Immediate ENOEXEC did not restore previous files')
            require((target/'plugin.mjs').is_file() and not(target/'broken').exists(),'Failed replacement files remained active')
            require(plugin(host,'com.elgato.counter')['pid']==counter_pid,'Recovery disrupted Counter')
            apply(host,prepare(host,v2));version(host,'2.0.0')
            saved=identities(host,[a,b])
        except Exception:
            print(host.log.read_text()[-7000:]);raise
        finally:host.stop()
        host=Host(binary,[],state,root/'restart.log')
        try:
            host.ready();version(host,'2.0.0')
            require(plugin(host)['rollback_available'] and plugin(host)['rollback_version']=='1.0.0','Restart lost journal-based rollback discovery')
            require(identities(host,[a,b])==saved,'Restart changed update settings/contexts')
            survivor=plugin(host,'com.elgato.counter')['pid']
            apply(host,prepare(host,kind='rollback'));version(host,'1.0.0')
            require(plugin(host,'com.elgato.counter')['pid']==survivor,'Recovered rollback restarted survivor')
            wait(host,lambda:all(any(r['kind']=='willAppear' and r['pid']==plugin(host)['pid'] and r.get('context')==context and r.get('payload',{}).get('settings',{}).get('keep')==keep for r in rows(events)) for context,keep in [(first,'first'),(second,'second')]),'rollback replays retained contexts')
            # Create another real v1→v2 transaction, then damage only its current
            # manifest after stopping this owned Host. Directory identity stays
            # unchanged, so recovery must find the validated prior version.
            apply(host,prepare(host,v2));version(host,'2.0.0')
            damaged_pid=plugin(host)['pid']
            before_damage=identities(host,[a,b])
            report={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'review_cancel_wrong_uuid_and_stale_ticket_nondisruptive':True,'confirmed_atomic_update_and_rollback':True,'executed_version_markers_v2_then_v1':True,'survivor_counter_pid_settings_retained':True,'two_device_contexts_preserved':True,'removed_action_unavailable_then_restored':True,'disabled_update_and_rollback_never_launch':True,'immediate_enoexec_restores_previous_files_and_runtime':True,'rollback_discovered_after_host_restart':True,'boundary':'Managed temporary plugin packages, generated generic-ws fixture and unchanged Counter. Broken candidate has only a malformed ELF header; kernel rejects execution. No user state, external service, audio, keyboard or USB.'}
        finally:host.stop()
        require(not Path(f'/proc/{damaged_pid}').exists(),'Owned v2 process remains before corruption')
        events_before_damage=len(rows(events))
        (target/'manifest.json').write_text('{ intentionally damaged current manifest')
        host=Host(binary,[],state,root/'damaged-restart.log')
        try:
            host.ready()
            host.wait(lambda s:any(p['id']=='com.elgato.counter' and p['connected'] for p in s['plugins']),'Counter starts beside damaged current plugin')
            damaged=next(p for p in host.state()['plugins'] if Path(p['path'])==target)
            require(damaged['id']!=PLUGIN and damaged['version'] is None,'Damaged plugin did not use unavailable placeholder identity')
            require(damaged['pid'] is None and not damaged['connected'],'Damaged current plugin was launched')
            require(damaged['rollback_available'] and damaged['rollback_version']=='1.0.0','Damaged current manifest hid validated prior version')
            require(len(rows(events))==events_before_damage,'Damaged current plugin executed during restart')
            require(identities(host,[a,b])==before_damage,'Damaged restart discarded assignments/settings')
            survivor=plugin(host,'com.elgato.counter')['pid']
            count=next(s for s in inspect(host,a)['slots'] if s['context']==counter)['settings']['count']
            press(host,a,2)
            wait(host,lambda:next(s for s in inspect(host,a)['slots'] if s['context']==counter)['settings']['count']==count+1,'Counter works while current plugin is damaged')
            before_repair=identities(host,[a,b])
            repair=prepare(host,kind='rollback',selection=damaged)
            require(repair['id']==damaged['id'] and repair['new_version']=='1.0.0','Repair review lost selected placeholder identity or prior version')
            require(plugin(host,'com.elgato.counter')['pid']==survivor,'Repair preparation restarted survivor')
            apply(host,repair);version(host,'1.0.0')
            require(not any(p['id']==damaged['id'] for p in host.state()['plugins']),'Repair retained synthetic unavailable identity')
            require(plugin(host,'com.elgato.counter')['pid']==survivor,'Repair restarted survivor')
            require(identities(host,[a,b])==before_repair,'Repair changed retained settings/contexts')
            repaired_pid=plugin(host)['pid']
            wait(host,lambda:all(any(r['kind']=='willAppear' and r['pid']==repaired_pid and r.get('context')==context and r.get('payload',{}).get('settings',{}).get('keep')==keep for r in rows(events)) for context,keep in [(first,'first'),(second,'second')]),'repaired v1 replays retained contexts')
            require(manifest_version(target)=='1.0.0','Repair did not restore readable original manifest')
            require(all(next(s for s in inspect(host,device)['slots'] if s['context']==context)['available'] for device,context in [(a,first),(a,legacy),(b,second)]),'Repaired actions remain unavailable')
            report['damaged_current_never_launched']=True
            report['damaged_restart_repair_via_synthetic_identity']=True
            report['repair_preserves_survivor_pid_and_assigned_contexts']=True
        except Exception:
            print(host.log.read_text()[-7000:]);raise
        finally:host.stop()
    args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: managed review/update/restore, no-disruption preflight/cancel/stale guards, disabled preservation, ENOEXEC recovery restart rollback discovery and damaged-manifest repair.')

if __name__=='__main__':main()
