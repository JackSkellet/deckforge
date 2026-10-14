// @vitest-environment jsdom
import { useState } from 'react';
import { afterEach,expect,it,vi } from 'vitest';
import {cleanup,fireEvent,render,screen,within} from '@testing-library/react';
import {Sequence} from './Sequence';
import {children,TRIGGER,type Action,type RecordValue} from './model';
const multi='com.elgato.streamdeck.multiactions.routine';
const base:Action={uuid:'a',name:'Alpha',controllers:['Keypad'],supported_in_multi:true,defaults:{kept:7},plugin:'fixture',plugin_name:'Fixture',category:'Fixture',visible:true,multi_only:false,states:[]};
const actions:Action[]=[base,{...base,uuid:'b',name:'Beta',defaults:{}},{...base,uuid:'dial',name:'Dial',controllers:['Encoder']}];
function transfer(uuid?:string){const data:Record<string,string>=uuid?{'application/x-deckforge-action':uuid}:{};return {get types(){return Object.keys(data)},getData:(key:string)=>data[key]??'',setData:(key:string,value:string)=>{data[key]=value},dropEffect:'none',effectAllowed:'all'};}
const before=(element:Element,data:ReturnType<typeof transfer>)=>{vi.spyOn(element,'getBoundingClientRect').mockReturnValue({top:100,height:40} as DOMRect);fireEvent.dragOver(element,{dataTransfer:data,clientY:105});fireEvent.drop(element,{dataTransfer:data,clientY:105});};
afterEach(cleanup);
it('adds library actions at the drop position and reorders existing identities without copying',()=>{
 let current:RecordValue={};const error=vi.fn();
 function Harness(){const [raw,setRaw]=useState<RecordValue>({Actions:[{Actions:[]}],Future:9});current=raw;return <Sequence raw={raw} uuid={multi} onChange={setRaw} actions={actions} outputs={[]} onError={error}/>;}
 const {container}=render(<Harness/>);
 fireEvent.drop(container.querySelector('.sequence-drop-target')!,{dataTransfer:transfer('a')});
 const first=children(current,multi,0)[0];
 before(container.querySelector('.sequence-row')!,transfer('b'));
 expect(children(current,multi,0).map(x=>x.UUID)).toEqual(['b','a']);
 const data=transfer();fireEvent.dragStart(screen.getByLabelText('Drag action 2 to reorder'),{dataTransfer:data});
 expect(data.effectAllowed).toBe('move');expect(data.getData('text/plain')).toBe('Alpha');
 before(container.querySelector('.sequence-row')!,data);
 expect(children(current,multi,0).map(x=>x.UUID)).toEqual(['a','b']);
 expect(children(current,multi,0)[0]).toEqual(first);expect(current.Future).toBe(9);expect(error).not.toHaveBeenCalled();
 fireEvent.drop(container.querySelector('.sequence-drop-target')!,{dataTransfer:transfer('dial')});
 expect(error).toHaveBeenCalled();expect(children(current,multi,0)).toHaveLength(2);
});
it('rejects stale moves and blocks drops through a disabled editor',()=>{
 const a={UUID:'a',ActionID:'stable-a',Name:'Alpha',Settings:{kept:7}};let current:RecordValue={};const error=vi.fn();
 function Harness(){const [raw,setRaw]=useState<RecordValue>({Actions:[{Actions:[a,{UUID:'b',ActionID:'stable-b',Name:'Beta'}]}]});current=raw;return <><button onClick={()=>setRaw({...raw,Actions:[{Actions:[{...a,Name:'Changed'},children(raw,multi,0)[1]]}]})}>Change source</button><Sequence raw={raw} uuid={multi} onChange={setRaw} actions={actions} outputs={[]} onError={error}/></>;}
 const {container,unmount}=render(<Harness/>);const data=transfer();fireEvent.dragStart(screen.getByLabelText('Drag action 1 to reorder'),{dataTransfer:data});fireEvent.click(screen.getByText('Change source'));fireEvent.drop(container.querySelector('.sequence-drop-target')!,{dataTransfer:data});
 expect(error).toHaveBeenCalled();expect(children(current,multi,0)[0].Name).toBe('Changed');unmount();
 const change=vi.fn();const view=render(<fieldset disabled><Sequence raw={{Actions:[{Actions:[]}]}} uuid={multi} onChange={change} actions={actions} outputs={[]} onError={vi.fn()}/></fieldset>);
 fireEvent.drop(view.container.querySelector('.sequence-drop-target')!,{dataTransfer:transfer('a')});expect(change).not.toHaveBeenCalled();
});
it('keeps nested drops in their own container and uses the selected switch branch',()=>{
 let current:RecordValue={};const inner={UUID:multi,ActionID:'inner',Name:'Inner',Settings:{},Actions:[{Actions:[]}]};const switchId=multi+'2';
 function Harness(){const [raw,setRaw]=useState<RecordValue>({Actions:[{Actions:[inner]},{Actions:[]}]});current=raw;return <Sequence raw={raw} uuid={switchId} onChange={setRaw} actions={actions} outputs={[]} onError={vi.fn()}/>;}
 render(<Harness/>);fireEvent.click(screen.getByRole('button',{name:'1 Inner'}));
 const regions=screen.getAllByRole('region',{name:'Action sequence'});const innerRegion=regions[1];fireEvent.drop(innerRegion.querySelector('.sequence-drop-target')!,{dataTransfer:transfer('a')});
 expect(children(current,switchId,0)).toHaveLength(1);expect(children(children(current,switchId,0)[0],multi,0)).toHaveLength(1);
 fireEvent.click(within(regions[0]).getByRole('button',{name:'Sequence 2'}));fireEvent.drop(regions[0].querySelector('.sequence-drop-target')!,{dataTransfer:transfer('b')});
 expect(children(current,switchId,1).map(x=>x.UUID)).toEqual(['b']);expect(children(current,switchId,0)).toHaveLength(1);
});
it('does not bubble a drop on a nested gesture editor into its parent sequence',()=>{
 const gesture=TRIGGER;let current:RecordValue={};const error=vi.fn();
 function Harness(){const [raw,setRaw]=useState<RecordValue>({Actions:[{Actions:[{UUID:gesture,ActionID:'gesture',Name:'Gesture',Settings:{},Actions:[]}]}]});current=raw;return <Sequence raw={raw} uuid={multi} onChange={setRaw} actions={actions} outputs={[]} onError={error}/>;}
 render(<Harness/>);fireEvent.click(screen.getByRole('button',{name:'1 Gesture'}));
 const inner=screen.getByRole('region',{name:'Action trigger'});fireEvent.drop(inner,{dataTransfer:transfer('a')});
 expect(error).toHaveBeenCalled();expect(children(current,multi,0)).toHaveLength(1);expect(children(current,multi,0)[0].Actions).toEqual([]);
 fireEvent.drop(inner.querySelector('.sequence-row')!,{dataTransfer:transfer('a')});
 expect(children(current,multi,0)).toHaveLength(1);expect(children(current,multi,0)[0].Actions[0].UUID).toBe('a');
});
