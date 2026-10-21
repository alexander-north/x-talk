import {describe,it,expect} from 'vitest';
import {paintOrb} from './orb-art';
describe('shared recording spectrum animation',()=>{
 it('renders identical rays for dictation, meeting and meeting preview',()=>{
  function bars(meeting:boolean,preview=false){
   const result:{color:string;width:number}[]=[];
   const context=new Proxy({fillStyle:'',fillRect(x:number,_y:number,width:number){if(x===119)result.push({color:this.fillStyle,width});}}, {
    get(target,key){if(key in target)return target[key as keyof typeof target];if(String(key).startsWith('create'))return ()=>({addColorStop(){}});return ()=>{};},
   }) as unknown as CanvasRenderingContext2D;
   paintOrb(context,preview?'idle':'recording',0,Array.from({length:48},(_,i)=>i/48),false,0,meeting,preview);
   return result;
  }
  const dictation=bars(false),meeting=bars(true);
  expect(meeting).toEqual(dictation);
  expect(bars(true,true)).toEqual(dictation);
  expect(new Set(dictation.map(bar=>bar.width)).size).toBeGreaterThan(5);
 });
});
