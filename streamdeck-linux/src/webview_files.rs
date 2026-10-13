//! Native inspector file selections with Stream Deck's full-path input values.
use gtk::prelude::*;
use javascriptcore::ValueExt;
use std::rc::Rc;
use webkit2gtk::{FileChooserRequest, FileChooserRequestExt, WebView, WebViewExt};

pub fn script(origin: &str) -> String {
    include_str!("webview_files.js").replace("__ORIGIN__", &serde_json::to_string(origin).unwrap())
}

type Complete = Box<dyn FnOnce(Option<Vec<String>>)>;
type Picker = Rc<dyn Fn(&WebView, &FileChooserRequest, Complete)>;

pub fn install(view: &WebView, origin: String) {
    install_picker(view, origin, Rc::new(pick));
}

fn install_picker(view: &WebView, origin: String, picker: Picker) {
    view.connect_run_file_chooser(move |view, request| {
        let uri = view.uri().map(|s| s.to_string()).unwrap_or_default();
        if !url::Url::parse(&uri).is_ok_and(|url| url[..url::Position::BeforePath] == origin) {
            // Remote pages keep ordinary WebKit file-input behavior.
            return false;
        }
        let request = request.clone();
        let view = view.clone();
        let picker = picker.clone();
        let browser = view.clone();
        view.evaluate_javascript(
            "window.__deckforgeFileChooser?.begin()",
            None,
            None,
            None::<&webkit2gtk::gio::Cancellable>,
            move |result| {
                let id = result
                    .ok()
                    .filter(|v| v.is_string())
                    .map(|v| v.to_str().to_string());
                let Some(id) = id else {
                    request.cancel();
                    return;
                };
                let target = browser.clone();
                let selected = request.clone();
                picker(
                    &browser,
                    &request,
                    Box::new(move |paths| {
                        if target.uri().as_deref() != Some(uri.as_str()) {
                            selected.cancel();
                            return;
                        }
                        let json = serde_json::to_string(&(id, &paths)).unwrap();
                        let script =
                            format!("window.__deckforgeFileChooser?.complete(...{json}) === true");
                        target.evaluate_javascript(
                            &script,
                            None,
                            None,
                            None::<&webkit2gtk::gio::Cancellable>,
                            move |result| {
                                if result.is_ok_and(|v| v.to_boolean())
                                    && let Some(paths) = paths
                                    && !paths.is_empty()
                                {
                                    selected.select_files(
                                        &paths.iter().map(String::as_str).collect::<Vec<_>>(),
                                    );
                                } else {
                                    selected.cancel();
                                }
                            },
                        );
                    }),
                );
            },
        );
        true
    });
}

fn pick(view: &WebView, request: &FileChooserRequest, complete: Complete) {
    let parent = view
        .toplevel()
        .and_then(|widget| widget.downcast::<gtk::Window>().ok());
    let dialog = gtk::FileChooserNative::new(
        Some("Choose a file"),
        parent.as_ref(),
        gtk::FileChooserAction::Open,
        Some("Open"),
        Some("Cancel"),
    );
    dialog.set_modal(true);
    dialog.set_select_multiple(request.selects_multiple());
    if let Some(filter) = request.mime_types_filter() {
        dialog.add_filter(filter.clone());
        dialog.set_filter(&filter);
    }
    if let Some(path) = request.selected_files().first() {
        dialog.set_filename(path.as_str());
    }
    // Native dialogs are not widgets; retain the object until its response.
    let keep_alive = std::cell::RefCell::new(Some(dialog.clone()));
    let complete = std::cell::RefCell::new(Some(complete));
    dialog.connect_response(move |dialog, response| {
        let paths = (response == gtk::ResponseType::Accept)
            .then(|| {
                dialog
                    .filenames()
                    .iter()
                    .map(|p| p.to_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .flatten();
        if let Some(complete) = complete.borrow_mut().take() {
            complete(paths);
        }
        dialog.destroy();
        keep_alive.borrow_mut().take();
    });
    dialog.show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        fs,
        time::{Duration, Instant},
    };
    use wry::{WebViewBuilder, WebViewBuilderExtUnix};

    #[test]
    #[ignore = "opens an isolated native WebKit window; run explicitly with a desktop session"]
    fn native_file_controls_return_selected_paths_and_readable_files() {
        gtk::init().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugin");
        fs::create_dir(&root).unwrap();
        // Unchanged retained SDPI component; application code is never copied from it.
        let component = std::path::Path::new(
            "../evidence/official-samples/data-sources/com.elgato.product-viewer.sdPlugin/ui/sdpi-components.js",
        );
        let bytes = fs::read(component).expect("Retained SDPI component fixture required");
        use sha2::Digest;
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(&bytes)),
            "f6c0dfd2ed68e18084b9952842b86e3850cf837d674704700c2a0718e0a24f6b"
        );
        fs::write(root.join("sdpi-components.js"), bytes).unwrap();
        fs::write(root.join("frame.html"), "<input type='file' id='file'>").unwrap();
        let selected = temp.path().join("chosen æ 50% #.txt");
        fs::write(&selected, "selected file content").unwrap();
        let other = temp.path().join("second.txt");
        fs::write(&other, "second content").unwrap();
        let script = r#"
<script src='sdpi-components.js'></script>
<form id='form'><input type='file' id='plain' accept='text/plain' multiple></form>
<sdpi-file id='component'></sdpi-file>
<iframe id='local' src='frame.html'></iframe>
<script>
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async fn=>{for(let n=0;n<150;n++){const v=fn();if(v)return v;await pause(20)}throw Error('Native file fixture timed out')};
const click=input=>{window.requestChoice=()=>input.click();window.ipc.postMessage(JSON.stringify({choose:true}));};
const select=async input=>{const done=new Promise(resolve=>input.addEventListener('change',resolve,{once:true}));click(input);await done;};
window.runFileFixture=async()=>{
 const input=document.getElementById('plain');
 await select(input);
 const single={value:decodeURIComponent(input.value),count:input.files.length,text:await input.files[0].text(),second:await input.files[1].text()};
 const before=input.value;
 const cancelled=new Promise(resolve=>input.addEventListener('cancel',resolve,{once:true}));click(input);await cancelled;
 if(input.value!==before)throw Error('Cancel erased selection');
 document.getElementById('form').reset();await pause(0);
 if(input.value!==''||input.files.length)throw Error('Reset retained selection');
 const component=document.getElementById('component');
 const child=await until(()=>component.shadowRoot?.querySelector('input[type=file]'));
 await select(child);await pause(0);
 const result={single,componentValue:component.value,childValue:decodeURIComponent(child.value),childText:await child.files[0].text(),cancelPreserved:true,resetCleared:true};
 const transfer=new DataTransfer();transfer.items.add(new File(['synthetic'],'synthetic.txt'));child.files=transfer.files;
 if(decodeURIComponent(child.value)===result.childValue)throw Error('Programmatic file change reused selected path');
 child.value='';if(child.value!==''||child.files.length)throw Error('Clear retained selection');
 const framed=document.getElementById('local').contentDocument.getElementById('file');
 await select(framed);result.frameValue=decodeURIComponent(framed.value);
 if(window.__deckforgeFileChooser.complete('stale-request',['unselected']))throw Error('Unknown chooser accepted');
 const remote=document.createElement('iframe');
 const remoteResult=new Promise(resolve=>window.addEventListener('message',event=>{if(event.source===remote.contentWindow)resolve(event.data)}));
 remote.src='data:text/html,'+encodeURIComponent('<script>parent.postMessage({adapter:typeof window.__deckforgeFileChooser},"*")<'+ '/script>');document.body.appendChild(remote);
 if((await remoteResult).adapter!=='undefined')throw Error('Adapter injected into unrelated frame');
 result.foreignFrameIsolated=true;
 window.ipc.postMessage(JSON.stringify(result));
};
</script>"#;
        let entry = root.join("index.html");
        fs::write(&entry, script).unwrap();
        let assets = crate::webview_assets::Assets::new(&root, &entry).unwrap();
        let url = assets.url().clone();
        let origin = url[..url::Position::BeforePath].to_owned();
        let result = Rc::new(RefCell::new(None));
        let response = result.clone();
        let choose = Rc::new(RefCell::new(false));
        let trigger = choose.clone();
        let loaded = Rc::new(RefCell::new(false));
        let ready = loaded.clone();
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&container);
        let mut context = wry::WebContext::new(Some(temp.path().join("browser")));
        let browser = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol(crate::webview_assets::SCHEME.into(), move |_, request| {
                assets.respond(request)
            })
            .with_initialization_script_for_main_only(super::script(&origin), false)
            .with_on_page_load_handler(move |event, _| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    *ready.borrow_mut() = true;
                }
            })
            .with_ipc_handler(move |request| {
                let value = serde_json::from_str::<serde_json::Value>(request.body()).unwrap();
                if value["choose"] == true {
                    *trigger.borrow_mut() = true;
                } else {
                    *response.borrow_mut() = Some(value);
                }
            })
            .build_gtk(&container)
            .unwrap();
        use wry::WebViewExtUnix;
        let choices = Rc::new(RefCell::new(std::collections::VecDeque::from([
            Some(vec![
                selected.to_str().unwrap().to_owned(),
                other.to_str().unwrap().to_owned(),
            ]),
            None,
            Some(vec![selected.to_str().unwrap().to_owned()]),
            Some(vec![selected.to_str().unwrap().to_owned()]),
        ])));
        let queue = choices.clone();
        install_picker(
            &browser.webview(),
            origin,
            Rc::new(move |_, request, complete| {
                let paths = queue
                    .borrow_mut()
                    .pop_front()
                    .expect("Unexpected native chooser");
                if queue.borrow().len() == 3 {
                    assert!(request.selects_multiple());
                    assert!(request.mime_types().iter().any(|s| s == "text/plain"));
                }
                complete(paths);
            }),
        );
        window.show_all();
        browser.load_url(url.as_str()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut started = false;
        while result.borrow().is_none() && Instant::now() < deadline {
            if *choose.borrow() {
                *choose.borrow_mut() = false;
                browser.evaluate_script("requestChoice()").unwrap();
            }
            if *loaded.borrow() && !started {
                started = true;
                browser.evaluate_script("runFileFixture().catch(error=>window.ipc.postMessage(JSON.stringify({error:String(error)})))").unwrap();
            }
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        window.close();
        let value = result.borrow().clone().unwrap_or_else(|| {
            panic!(
                "Native file control did not complete: loaded={started}, remaining={}",
                choices.borrow().len()
            )
        });
        assert!(value.get("error").is_none(), "{value}");
        assert_eq!(value["single"]["value"], selected.to_str().unwrap());
        assert_eq!(value["single"]["count"], 2);
        assert_eq!(value["single"]["text"], "selected file content");
        assert_eq!(value["single"]["second"], "second content");
        assert_eq!(value["componentValue"], selected.to_str().unwrap());
        assert_eq!(value["childValue"], selected.to_str().unwrap());
        assert_eq!(value["childText"], "selected file content");
        assert_eq!(value["frameValue"], selected.to_str().unwrap());
        assert_eq!(value["foreignFrameIsolated"], true);
        assert!(choices.borrow().is_empty());
    }
}
