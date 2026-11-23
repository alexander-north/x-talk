import {describe,it,expect} from 'vitest';
import {capturedShortcut,describeShortcut,formatShortcut,normalizeShortcut,shortcutExample,shortcutTitle} from './shortcuts';
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
describe('shortcut labels',()=>{
 it('stores Mac modifier names as the canonical tokens the desktop parser registers',()=>{
  expect(normalizeShortcut('Cmd+Option+K')).toBe('Super+Alt+K');
  expect(normalizeShortcut(' control + option + Space ')).toBe('Ctrl+Alt+Space');
  expect(normalizeShortcut('⌃+⌥+⇧+⌘+F7')).toBe('Ctrl+Alt+Shift+Super+F7');
  expect(normalizeShortcut('Command+Shift+Equal')).toBe('Super+Shift+Equal');
  expect(normalizeShortcut('CmdOrCtrl+K')).toBe('CmdOrCtrl+K');
  expect(normalizeShortcut('F12')).toBe('F12');expect(normalizeShortcut('')).toBe('');
 });
 it('shows Mac symbols and names on macOS and leaves Windows labels unchanged',()=>{
  expect(formatShortcut('Ctrl+Alt+Space','macos')).toBe('⌃⌥Space');
  expect(formatShortcut('Ctrl+Alt+Shift+Super+F7','macos')).toBe('⌃⌥⇧⌘F7');
  expect(describeShortcut('Ctrl+Alt+Escape','macos')).toBe('Control+Option+Escape');
  expect(describeShortcut('Cmd+Shift+K','macos')).toBe('Command+Shift+K');
  expect(shortcutTitle('Ctrl+Alt+Space','macos')).toBe('Control+Option+Space');
  expect(formatShortcut('F9','macos')).toBe('F9');
  for(const value of ['Ctrl+Alt+Space','Super+7',''])expect([formatShortcut(value,'windows'),describeShortcut(value,'windows')]).toEqual([value,value]);
  expect(shortcutTitle('Ctrl+Alt+Space','windows')).toBeUndefined();expect(shortcutTitle('','macos')).toBeUndefined();
  expect(shortcutExample('macos')).toBe('Control+Option+Space');expect(shortcutExample('windows')).toBe('Ctrl+Alt+Space');
 });
});
