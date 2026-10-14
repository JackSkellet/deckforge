#!/usr/bin/env python3
"""Observe a short-lived owned GTK window through the actual KDE session bus."""
import argparse
import json
import os
import shlex
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

SOURCE=r'''
#include <gtk/gtk.h>
#include <stdio.h>
static gboolean done(gpointer data){gtk_main_quit();return G_SOURCE_REMOVE;}
int main(int argc,char **argv){
 g_set_prgname("deckforge-focus-fixture");gtk_init(&argc,&argv);
 GtkWidget *w=gtk_window_new(GTK_WINDOW_TOPLEVEL);
 gtk_window_set_title(GTK_WINDOW(w),"Deckforge focus fixture");
 gtk_window_set_default_size(GTK_WINDOW(w),320,100);
 gtk_container_add(GTK_CONTAINER(w),gtk_label_new("Owned focus-observer test window"));
 g_signal_connect(w,"destroy",G_CALLBACK(gtk_main_quit),NULL);
 gtk_widget_show_all(w);gtk_window_present(GTK_WINDOW(w));
 puts("ready");fflush(stdout);g_timeout_add(2000,done,NULL);gtk_main();return 0;
}
'''

def wait(predicate,label,seconds=5):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        found=predicate()
        if found:return found
        time.sleep(.04)
    raise AssertionError('Timed out: '+label)

def records(path):
    if not path.exists():return []
    result=[]
    for line in path.read_text().splitlines():
        try:result.append(json.loads(line))
        except json.JSONDecodeError:pass
    return result

def dbus(path,method,*args):
    return subprocess.run(['qdbus6','org.kde.KWin',path,method,*map(str,args)],capture_output=True,text=True,check=True,timeout=4).stdout.strip()

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/smart-profiles/focus-native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'summary.json').unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-focus-fixture-') as d:
        root=Path(d);source=root/'fixture.c';source.write_text(SOURCE);binary=root/'focus-fixture'
        flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','gtk+-3.0'],text=True))
        subprocess.run(['cc',str(source),'-o',str(binary),*flags],check=True,capture_output=True)
        log=a.output/'events.jsonl';err=a.output/'observer.log';window=None;script_name=None
        with log.open('w') as output,err.open('w') as errors:
            probe=subprocess.Popen([str(a.binary.resolve()),'focus-probe','--seconds','8','--events'],stdout=output,stderr=errors)
            try:
                ready=wait(lambda:next((r for r in records(log) if r['snapshot']['error'] is None),None),'observer ready')
                with (a.output/'fixture.log').open('w') as fixture_log:
                    window=subprocess.Popen([str(binary)],stdout=fixture_log,stderr=fixture_log)
                    wait(lambda:'ready' in (a.output/'fixture.log').read_text(),'owned window ready')
                    assert window.poll() is None
                    # Activate only the still-live PID created by this fixture.
                    script_name='deckforge-focus-test-'+str(uuid.uuid4());script=root/'activate.js'
                    script.write_text(f'for(const w of workspace.windowList()){{if(w.pid==={window.pid} && w.normalWindow){{workspace.activeWindow=w;break;}}}}')
                    identifier=int(dbus('/Scripting','org.kde.kwin.Scripting.loadScript',script,script_name));assert identifier>=0
                    try:dbus(f'/Scripting/Script{identifier}','org.kde.kwin.Script.run')
                    finally:
                        dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',script_name)
                        assert dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',script_name)=='false'
                        script_name=None
                    focused=wait(lambda:next((r for r in records(log) if (r['snapshot']['focus'] or {}).get('pid')==window.pid),None),'owned application focus')
                    assert focused['snapshot']['focus']['executable']==str(binary)
                    assert 'title' not in focused['snapshot']['focus'] and 'caption' not in focused['snapshot']['focus']
                    window.wait(timeout=4)
                    wait(lambda:next((r for r in records(log) if r['snapshot']['revision']>focused['snapshot']['revision'] and (r['snapshot']['focus'] or {}).get('pid')!=window.pid),None),'focus departure')
                assert probe.wait(timeout=10)==0
                final=records(log)[-1];assert final['stopped']
                assert dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',ready['script'])=='false'
            finally:
                if window is not None and window.poll() is None:window.terminate();window.wait(timeout=3)
                if script_name:dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',script_name)
                # Let the bounded observer finish and unload its own script.
                if probe.poll() is None:probe.wait(timeout=12)
        env=os.environ.copy();env['DBUS_SESSION_BUS_ADDRESS']='unix:path='+str(root/'missing-bus')
        unavailable=subprocess.run([str(a.binary.resolve()),'focus-probe','--seconds','1'],env=env,capture_output=True,text=True,timeout=5)
        assert unavailable.returncode!=0 and json.loads(unavailable.stdout)['snapshot']['error']
    report={'backend':'KDE KWin','session':os.environ.get('XDG_SESSION_TYPE'),'real_focus_event':True,'owned_window_only':True,
        'executable_verified':True,'focus_departure':True,'script_unloaded':True,'missing_bus_fails_closed':True,'no_titles_or_contents':True,'profiles_changed':False,'usb_access':False}
    (a.output/'summary.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))

if __name__=='__main__':main()
