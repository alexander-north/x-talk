import {platform as currentPlatform,type Platform} from './platform';
/** Use physical key codes, matching the desktop global shortcut parser. */
export function capturedShortcut(event: Pick<KeyboardEvent,'code'|'ctrlKey'|'altKey'|'shiftKey'|'metaKey'|'repeat'|'isComposing'>): string | null {
 if(event.repeat||event.isComposing)return null;
 const code=event.code;
 if(!/^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-9]|2[0-4])|Space|Enter|Escape|Tab|Backspace|Delete|Insert|Home|End|PageUp|PageDown|Arrow(Up|Down|Left|Right)|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash|Numpad([0-9]|Add|Subtract|Multiply|Divide|Decimal|Enter))$/.test(code))return null;
 const modifiers=[event.ctrlKey?'Ctrl':'',event.altKey?'Alt':'',event.shiftKey?'Shift':'',event.metaKey?'Super':''].filter(Boolean);
 return [...modifiers,code.replace(/^Key|^Digit/,'')].join('+');
}
// Stored shortcuts keep the canonical Ctrl/Alt/Shift/Super tokens on every platform; the
// global shortcut parser also accepts the Mac names, so manual entry may use either.
const aliases:Record<string,string>={ctrl:'Ctrl',control:'Ctrl','⌃':'Ctrl',alt:'Alt',option:'Alt','⌥':'Alt',shift:'Shift','⇧':'Shift',super:'Super',cmd:'Super',command:'Super','⌘':'Super'};
/** Rewrites modifier names (Cmd, Option, Control…) to the stored canonical tokens; keys are kept as typed. */
export function normalizeShortcut(shortcut:string):string{
 const tokens=shortcut.split('+').map(t=>t.trim());
 if(tokens.length<2)return shortcut.trim();
 return tokens.map((t,i)=>i<tokens.length-1?aliases[t.toLowerCase()]??t:t).join('+');
}
const macNames:Record<string,[string,string]>={Ctrl:['⌃','Control'],Alt:['⌥','Option'],Shift:['⇧','Shift'],Super:['⌘','Command']};
function macParts(shortcut:string){
 const tokens=normalizeShortcut(shortcut).split('+');
 return {modifiers:tokens.slice(0,-1).map(t=>macNames[t]??[t,t]),key:tokens[tokens.length-1]};
}
/** Compact label for a stored shortcut: Mac modifier symbols on macOS, the stored text elsewhere. */
export function formatShortcut(shortcut:string,platform:Platform=currentPlatform):string{
 if(platform!=='macos'||!shortcut.includes('+'))return shortcut;
 const {modifiers,key}=macParts(shortcut);
 return modifiers.map(m=>m[0]).join('')+key;
}
/** Spelled-out label for tooltips and assistive technology. */
export function describeShortcut(shortcut:string,platform:Platform=currentPlatform):string{
 if(platform!=='macos'||!shortcut.includes('+'))return shortcut;
 const {modifiers,key}=macParts(shortcut);
 return [...modifiers.map(m=>m[1]),key].join('+');
}
/** Tooltip for a compact Mac label; none elsewhere, where the label is already spelled out. */
export function shortcutTitle(shortcut:string,platform:Platform=currentPlatform):string|undefined{return platform==='macos'&&shortcut?describeShortcut(shortcut,platform):undefined;}
/** Example shown in the manual entry field. */
export function shortcutExample(platform:Platform=currentPlatform):string{return platform==='macos'?'Control+Option+Space':'Ctrl+Alt+Space';}
