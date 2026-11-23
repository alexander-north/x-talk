/** The one place that decides which desktop platform the UI runs on. */
export type Platform='macos'|'windows'|'other';
type NavigatorLike={userAgent?:string;platform?:string;userAgentData?:{platform?:string}};
export function detectPlatform(nav:NavigatorLike|undefined=globalThis.navigator as NavigatorLike|undefined):Platform{
 const hint=[nav?.userAgentData?.platform,nav?.platform,nav?.userAgent].filter(Boolean).join(' ');
 // WKWebView reports "Macintosh; Intel Mac OS X" on Apple Silicon too; WebView2 reports "Windows NT".
 if(/Mac|Macintosh/i.test(hint)&&!/iPhone|iPad/i.test(hint))return 'macos';
 if(/\bWin/i.test(hint))return 'windows';
 return 'other';
}
export const platform:Platform=detectPlatform();
export const isMac=platform==='macos';
