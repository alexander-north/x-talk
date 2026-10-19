/** Use physical key codes, matching the desktop global shortcut parser. */
export function capturedShortcut(event: Pick<KeyboardEvent,'code'|'ctrlKey'|'altKey'|'shiftKey'|'metaKey'|'repeat'|'isComposing'>): string | null {
 if(event.repeat||event.isComposing)return null;
 const code=event.code;
 if(!/^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-9]|2[0-4])|Space|Enter|Escape|Tab|Backspace|Delete|Insert|Home|End|PageUp|PageDown|Arrow(Up|Down|Left|Right)|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash|Numpad([0-9]|Add|Subtract|Multiply|Divide|Decimal|Enter))$/.test(code))return null;
 const modifiers=[event.ctrlKey?'Ctrl':'',event.altKey?'Alt':'',event.shiftKey?'Shift':'',event.metaKey?'Super':''].filter(Boolean);
 return [...modifiers,code.replace(/^Key|^Digit/,'')].join('+');
}
