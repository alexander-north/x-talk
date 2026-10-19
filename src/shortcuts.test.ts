import {describe,it,expect} from 'vitest';
import {capturedShortcut} from './shortcuts';
const key=(code:string,extra:Partial<KeyboardEvent>={})=>capturedShortcut({code,ctrlKey:false,altKey:false,shiftKey:false,metaKey:false,repeat:false,isComposing:false,...extra});
describe('shortcut capture',()=>{
 it('captures modifiers and physical keys without shifted punctuation ambiguity',()=>{
  expect(key('Space',{ctrlKey:true,altKey:true})).toBe('Ctrl+Alt+Space');
  expect(key('KeyA',{ctrlKey:true,shiftKey:true})).toBe('Ctrl+Shift+A');
  expect(key('Equal',{ctrlKey:true,shiftKey:true})).toBe('Ctrl+Shift+Equal');
  expect(key('Digit7',{metaKey:true})).toBe('Super+7');
 });
 it('supports function, navigation, escape, and numpad keys',()=>{
  for(const code of ['F12','ArrowLeft','Escape','NumpadAdd','NumpadEnter'])expect(key(code)).toBe(code);
 });
 it('ignores modifier-only, repeat, composing, and unsupported events',()=>{
  for(const code of ['ControlLeft','ShiftRight','AltLeft','MetaLeft','Unidentified','AudioVolumeUp'])expect(key(code)).toBeNull();
  expect(key('KeyA',{repeat:true})).toBeNull();expect(key('KeyA',{isComposing:true})).toBeNull();
 });
});
