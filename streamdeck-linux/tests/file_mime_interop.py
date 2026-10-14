#!/usr/bin/env python3
"""Build an isolated MIME database and verify default-handler preservation."""
import importlib.util
import json
import os
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path
from unittest.mock import patch

root=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('desktop_installer',root/'scripts/install-desktop.py')
installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
with patch.object(installer.shutil,'which',return_value='/usr/bin/xdg-mime'):
    with patch.object(installer.subprocess,'run',return_value=subprocess.CompletedProcess([],0,'other.desktop\n','')) as run:
        assert 'preserved' in installer.register_file_handler('application/x-streamdeck-action')
        assert run.call_count==1
    with patch.object(installer.subprocess,'run',return_value=subprocess.CompletedProcess([],0,'','')) as run:
        assert 'registered' in installer.register_file_handler('application/x-streamdeck-action')
        assert run.call_args.args[0]==['xdg-mime','default','deckforge.desktop','application/x-streamdeck-action']
with tempfile.TemporaryDirectory(prefix='deckforge-mime-') as d:
    directory=Path(d);data=directory/'data';packages=data/'mime/packages';packages.mkdir(parents=True)
    shutil.copyfile(root/'assets/deckforge-mime.xml',packages/'deckforge.xml')
    subprocess.run(['update-mime-database',str(data/'mime')],check=True,capture_output=True,text=True)
    env=os.environ.copy();env['XDG_DATA_HOME']=str(data)
    verified=[]
    for extension,mime in [('streamDeckAction','application/x-streamdeck-action'),('STREAMDECKPROFILE','application/x-streamdeck-profile'),('streamDeckPlugin','application/x-streamdeck-plugin'),('STREAMDECKICONPACK','application/x-streamdeck-iconpack')]:
        path=directory/f'quoted % # file.{extension}'
        with zipfile.ZipFile(path,'w') as z:z.writestr('manifest.json','{}')
        result=subprocess.run(['gio','info','-a','standard::content-type',str(path)],env=env,check=True,capture_output=True,text=True)
        assert f'standard::content-type: {mime}' in result.stdout,result.stdout
        verified.append(mime)
    output=root/'target/file-open';output.mkdir(parents=True,exist_ok=True)
    (output/'mime.json').write_text(json.dumps({'types':verified,'existing_defaults_preserved':True,'isolated_database':True},indent=2)+'\n')
print('PASS: GIO recognizes action/profile/plugin/icon-pack ZIPs, mixed-case extensions, existing defaults preserved; isolated MIME database.')
