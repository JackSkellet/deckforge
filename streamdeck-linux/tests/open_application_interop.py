#!/usr/bin/env python3
"""Open Application through the real host, using only uniquely owned processes."""
import argparse,json,shutil,subprocess,tempfile,time
from pathlib import Path
from system_interop import Host
from sample_interop import slot,require
UUID='com.elgato.streamdeck.system.openapp'

def until(fn, name, seconds=6):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        if fn():return
        time.sleep(.03)
    raise AssertionError(name)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));parser.add_argument('--kwin',action='store_true');parser.add_argument('--report',type=Path,default=Path('target/open-application/process.json'));args=parser.parse_args()
    args.report.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-openapp-') as d:
        root=Path(d);host=Host(args.binary.resolve(),[],root/'state',root/'host.log');children=[]
        def input(event):host.post({'command':'input','event':event,'index':0})
        def press():input('keyDown');input('keyUp')
        def settings(value):host.post({'command':'settings','context':slot(host.state(),UUID)['context'],'settings':value})
        report={'kwin_requested':args.kwin}
        try:
            host.ready();host.post({'command':'assign','controller':'Keypad','index':0,'uuid':UUID})
            executable=root/'owned sleep';shutil.copy2('/usr/bin/sleep',executable)
            child=subprocess.Popen([str(executable),'30']);children.append(child)
            settings({'exec':str(executable),'args':'30','bring_to_front':False,'long_press':'force_kill','Future':7})
            input('keyDown');time.sleep(.3);require(child.poll() is None,'Hold fired before 500ms')
            until(lambda:child.poll() is not None,'500ms force close did not fire');require(child.returncode==-9,'Wrong signal');input('keyUp');time.sleep(.15)
            report['hold_fires_before_release']=True
            settings({'exec':str(executable),'args':'30','bring_to_front':False,'long_press':'none'})
            input('keyDown');time.sleep(.65);input('keyUp')
            require('launch requested' not in host.state()['system_status'].lower(),'None hold launched')
            report['none_consumes_release']=True
            if args.kwin:
                python=root/'owned-python';shutil.copy2('/usr/bin/python3',python)
                fixture=root/'window.py';events=root/'events'
                fixture.write_text('''import sys,os,json
from pathlib import Path
from PySide6.QtWidgets import QApplication,QWidget
from PySide6.QtCore import QEvent,QTimer
p=Path(sys.argv[1])
def record(value):
 with p.open('a') as f:f.write(value+'\\n')
class Window(QWidget):
 def event(self,event):
  if event.type()==QEvent.Type.WindowActivate:record('active')
  return super().event(event)
 def closeEvent(self,event):record('closed');event.accept()
app=QApplication([]);w=Window();w.setWindowTitle('Deckforge owned activation test');w.resize(300,100);w.showMinimized()
QTimer.singleShot(200,lambda:record('ready:'+str(os.getpid())))
QTimer.singleShot(15000,app.quit)
app.exec()
''')
                config={'exec':str(python),'args':f'"{fixture}" "{events}"','bring_to_front':True,'long_press':'quit','Future':{'retained':True}}
                settings(config);press()
                until(lambda:events.exists() and 'ready:' in events.read_text(),'Owned Qt window not ready')
                pid=int(next(line.split(':')[1] for line in events.read_text().splitlines() if line.startswith('ready:')))
                before=events.read_text().count('active')
                press();host.wait(lambda s:'brought to front' in s['system_status'],'KWin activation acknowledgement')
                until(lambda:events.read_text().count('active')>before,'Window was not activated')
                require(events.read_text().count('ready:')==1,'Activation launched another copy')
                report['actual_kwin_activation']=True
                page=host.state()['library']['active_page'];input('keyDown');host.post({'command':'page_create','name':'Away'})
                time.sleep(.65);input('keyUp');require('closed' not in events.read_text(),'Leaving page failed to cancel hold')
                host.post({'command':'page_select','id':page});input('keyDown')
                until(lambda:'closed' in events.read_text(),'Held close did not reach native window')
                input('keyUp');time.sleep(.15);require(events.read_text().count('ready:')==1,'Held close release reopened app')
                report.update(actual_kwin_close=True,departing_hold_cancelled=True,no_duplicate_launch=True)
                host.stop();host=Host(args.binary.resolve(),[],root/'state',root/'restart.log');host.ready()
                require(slot(host.state(),UUID)['settings']==config,'Restart lost settings')
                report['restart_settings_preserved']=True
            report['ok']=True
        except Exception:
            print(host.log.read_text());raise
        finally:
            host.stop()
            for child in children:
                if child.poll() is None:child.terminate()
                child.wait(timeout=3)
            args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: Open Application hold timing, force close, none mode'+(', actual KDE activation/window close, cancellation and restart' if args.kwin else '')+'. Only owned fixture processes affected.')
if __name__=='__main__':main()
