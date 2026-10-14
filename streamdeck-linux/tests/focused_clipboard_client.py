#!/usr/bin/env python3
"""Owned focused GTK text client for the private-compositor clipboard fixture."""
import json,sys
from pathlib import Path
import gi
gi.require_version('Gtk','3.0')
from gi.repository import Gtk,Gdk,GLib
root=Path(sys.argv[1]);assert str(root).startswith('/tmp/deckforge-focused-')
window=Gtk.Window(title='Deckforge private clipboard fixture');view=Gtk.TextView();window.add(view)
window.set_default_size(500,300);window.show_all();view.grab_focus();buffer=view.get_buffer();last=None
clipboard=Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
def poll():
    global last
    try:
        request=json.loads((root/'client-command.json').read_text())
        if request['id']==last:return True
        last=request['id']
        if request['operation']=='seed':
            buffer.set_text(request['text']);view.grab_focus()
            if request.get('select'):buffer.select_range(buffer.get_start_iter(),buffer.get_end_iter())
        elif request['operation']=='clipboard':clipboard.set_text(request['text'],-1)
        elif request['operation']!='read':raise AssertionError('Unknown fixture operation')
        value={'id':last,'text':buffer.get_text(buffer.get_start_iter(),buffer.get_end_iter(),True),'clipboard':clipboard.wait_for_text(),'active':window.is_active(),'focused':view.has_focus()}
        temporary=root/'client-state.tmp';temporary.write_text(json.dumps(value));temporary.replace(root/'client-state.json')
    except (OSError,ValueError):pass
    return True
GLib.timeout_add(5,poll)
GLib.timeout_add_seconds(60,lambda:Gtk.main_quit())
Gtk.main()
