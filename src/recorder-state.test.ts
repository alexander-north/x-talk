import {describe,it,expect} from 'vitest';
import {newCompletion,recorderPhase} from './recorder-state';
import type {Job} from './types';
const job=(status:string,stage='transcribing'):Job=>({id:'a',entry_id:'entry',seq:1,kind:'initial',stage,status,remote_id:null,error:null,delivery:'manual_only',eligible:true});
describe('recorder lifecycle',()=>{
 it('keeps the recorder occupied through transformation and cancellation',()=>{
  expect(recorderPhase(false,[job('running','transforming')],null,false)).toBe('transforming');
  expect(recorderPhase(false,[job('canceling','transforming')],null,true)).toBe('canceling');
  expect(recorderPhase(false,[],'importing',false)).toBe('transcribing');
  expect(recorderPhase(true,[],null,true)).toBe('recording');
 });
 it('celebrates only a newly completed operation, not saved history or an intermediate transcript',()=>{
  expect(newCompletion(new Map(),[job('complete','complete')])).toBeNull();
  expect(newCompletion(new Map([['a','running']]),[job('running','transforming')])).toBeNull();
  expect(newCompletion(new Map([['a','running']]),[job('complete','complete')])).toBe('a');
  expect(newCompletion(new Map([['a','running']]),[job('canceled')])).toBeNull();
  expect(newCompletion(new Map([['a','complete']]),[job('complete','complete')])).toBeNull();
 });
});
