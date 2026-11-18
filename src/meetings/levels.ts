/** Peak dBFS; this is a display transform, never recording gain. */
export function peakDb(peak:number):number {
 return Number.isFinite(peak)&&peak>0?Math.max(-60,Math.min(0,20*Math.log10(peak))):-60;
}
export function levelLabel(peak:number):string {return `${peakDb(peak).toFixed(0)} dBFS`;}
