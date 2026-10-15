#!/usr/bin/env python3
"""Native HTML plugin + setup inspector round trip without assigning any control."""
import argparse, hashlib, json, os, shutil, tempfile
from pathlib import Path
from sample_interop import RunningHost, require
from webview_interop import make_probe, wait, probe_events, PROBE_PLUGIN, PROBE_ACTION

def main():
    project=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    p.add_argument('--output',type=Path,default=project/'target/plugin-setup-interop')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-setup-') as d:
        root=Path(d)
        for key in ['XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_DATA_HOME','XDG_CACHE_HOME']:
            os.environ[key]=str(root/key)
        runtime=root/'runtime';runtime.mkdir(mode=0o700);os.environ['XDG_RUNTIME_DIR']=str(runtime)
        probe=make_probe(root/(PROBE_PLUGIN+'.sdPlugin'))
        other=make_probe(root/'other.sdPlugin')
        manifest=other/'manifest.json';manifest.write_text(manifest.read_text().replace(PROBE_PLUGIN,'com.deckforge.other-probe'))
        state=root/'state';host=RunningHost(a.binary.resolve(),[probe,other],state,a.output/'host.log')
        try:
            host.ready()
            for slot in host.state()['slots']:
                host.post({'command':'remove','context':slot['context']})
            before=host.state();require(not before['slots'],'Setup fixture must have no assigned controls')
            plugin=next(p for p in before['plugins'] if p['id']==PROBE_PLUGIN)
            require(plugin['setup_actions'][0]['uuid']==PROBE_ACTION,'Setup page was not advertised')
            host.post({'command':'plugin_setup','id':PROBE_PLUGIN,'uuid':PROBE_ACTION})
            result=wait(host,lambda s:any(e['kind']=='inspector_complete' for e in probe_events(s)),'setup inspector exchange')
            events=probe_events(result);complete=next(e for e in events if e['kind']=='inspector_complete')
            require(not result['slots'],'Setup assigned a physical control')
            host.post({'command':'close_inspector','context':complete['context']})
            wait(host,lambda s:not s['inspectors'],'setup closed')
            saved=json.loads((state/'session.json').read_text())
            require(saved['globals'][PROBE_PLUGIN]=={'fromPIGlobal':44,'fromPluginGlobal':45},'Shared settings not saved')
            require(complete['context'] not in json.dumps(saved),'Temporary action persisted')
            (a.output/'proof.json').write_text(json.dumps({'binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'native_html_plugin_and_inspector':True,'no_assigned_controls':True,'globals_persisted':True,'setup_context_not_persisted':True,'results':complete['results'],'usb_access':False},indent=2)+'\n')
        finally:host.stop()
    print('PASS: native setup inspector, shared settings persistence, temporary action settings, bidirectional plugin messages; no keys assigned.')
if __name__=='__main__':main()
