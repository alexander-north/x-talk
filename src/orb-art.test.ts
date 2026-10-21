import {describe,it,expect} from 'vitest';
import {paintOrb} from './orb-art';
describe('meeting orb source activity',()=>{
 it('keeps microphone pink and system gold bars independent',()=>{
  function bars(microphone:number,system:number,preview=false){
   const result:{color:string;width:number}[]=[];
   const context=new Proxy({fillStyle:'',fillRect(x:number,_y:number,width:number){if(x===119)result.push({color:this.fillStyle,width});}}, {
    get(target,key){if(key in target)return target[key as keyof typeof target];if(String(key).startsWith('create'))return ()=>({addColorStop(){}});return ()=>{};},
   }) as unknown as CanvasRenderingContext2D;
   paintOrb(context,preview?'idle':'recording',0,Array.from({length:48},(_,i)=>i%2===0?microphone:system),false,0,true,preview);
   return result;
  }
  const mic=bars(1,0),system=bars(0,1);
  expect(mic).toHaveLength(96);expect(mic[0].color).toContain('211,91,170');expect(mic[1].color).toContain('235,161,57');
  expect(mic[0].width).toBeGreaterThan(mic[1].width);
  expect(system[1].width).toBeGreaterThan(system[0].width);
  expect(bars(1,0,true)).toEqual(mic);
 });
});
