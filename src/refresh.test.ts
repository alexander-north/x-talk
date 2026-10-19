import {describe,it,expect} from 'vitest';
import {refreshQueue} from './refresh';
const deferred=()=>{let resolve!:()=>void;const promise=new Promise<void>(r=>resolve=r);return{promise,resolve};};
describe('progress refresh scheduling',()=>{
 it('coalesces a burst and never loses a later full configuration request',async()=>{
  const first=deferred();const calls:boolean[]=[];
  const refresh=refreshQueue(async full=>{calls.push(full);if(calls.length===1)await first.promise;});
  const a=refresh();await Promise.resolve();
  const b=refresh();const c=refresh(true);const d=refresh();
  expect(calls).toEqual([false]);first.resolve();await Promise.all([a,b,c,d]);
  expect(calls).toEqual([false,true]);
 });
 it('collapses same-tick updates and processes changes that arrive during the read',async()=>{
  const gate=deferred();const calls:boolean[]=[];
  const refresh=refreshQueue(async full=>{calls.push(full);if(calls.length===1)await gate.promise;});
  const a=refresh();const b=refresh();await Promise.resolve();const c=refresh();
  gate.resolve();await Promise.all([a,b,c]);expect(calls).toEqual([false,false]);
 });
 it('continues after a failed refresh',async()=>{
  let count=0;const refresh=refreshQueue(async()=>{if(++count===1)throw Error('read failed');});
  await expect(refresh()).rejects.toThrow('read failed');await refresh();expect(count).toBe(2);
 });
});
