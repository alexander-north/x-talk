import type {OrbPhase} from './recorder-state';
const tau=Math.PI*2;
const random=(seed:number)=>{const x=Math.sin(seed*127.1+311.7)*43758.5453;return x-Math.floor(x);};
function star(c:CanvasRenderingContext2D,x:number,y:number,r:number){
 c.beginPath();c.moveTo(x,y-r);c.quadraticCurveTo(x+r*.15,y-r*.15,x+r,y);c.quadraticCurveTo(x+r*.15,y+r*.15,x,y+r);c.quadraticCurveTo(x-r*.15,y+r*.15,x-r,y);c.quadraticCurveTo(x-r*.15,y-r*.15,x,y-r);c.fill();
}
/** A single bounded canvas keeps the glass, spectral halo and particles in one composited layer. */
export function paintOrb(c:CanvasRenderingContext2D,phase:OrbPhase,t:number,bands:number[],reduced:boolean,shapeMix=phase==='transforming'?1:0,meeting=false,previewActivity=false){
 c.clearRect(0,0,440,440);c.save();c.translate(220,220);
 const recording=phase==='recording'||meeting&&previewActivity&&phase==='idle',transcribing=phase==='transcribing'||phase==='canceling';
 const pulse=reduced?1:1+Math.sin(t*tau/4)*.012;
 const r=108*pulse,corner=r+(35-r)*shapeMix;
 const shape=(inset=0)=>{c.beginPath();c.roundRect(-r+inset,-r+inset,(r-inset)*2,(r-inset)*2,Math.max(20,corner-inset));};
 let glow=c.createRadialGradient(0,12,r*.7,0,12,157);glow.addColorStop(0,'#f7794938');glow.addColorStop(.65,'#e9884822');glow.addColorStop(1,'#d97f3800');c.fillStyle=glow;c.fillRect(-170,-170,340,340);
 // Warm glass body, with a directional champagne reflection and a burnt-orange lower edge.
 const glass=c.createLinearGradient(-90,-100,90,110);glass.addColorStop(0,meeting?'#ffe2f5':'#fff7d2');glass.addColorStop(.19,meeting?'#efa1d1':'#ffbd86');glass.addColorStop(.43,meeting?'#d96bac':'#f68b59');glass.addColorStop(.72,meeting?'#a74382':'#dc623c');glass.addColorStop(1,meeting?'#ffc78d':'#ffb881');
 c.shadowColor='#a33e302e';c.shadowBlur=24;c.shadowOffsetY=13;shape();c.fillStyle=glass;c.fill();c.shadowBlur=0;c.shadowOffsetY=0;
 const lens=c.createRadialGradient(-39,-47,1,8,15,127);lens.addColorStop(0,'#ffe9b498');lens.addColorStop(.42,'#ffbe6e28');lens.addColorStop(.8,'#bd3d3028');lens.addColorStop(1,'#ffdf9c70');shape(9);c.fillStyle=lens;c.fill();
 for(let i=0;i<6;i++){shape(i*2);c.lineWidth=i===0?1.6:1;c.strokeStyle=['#fff8e9dc','#ffc5a393','#fff7dba9','#d15b365c','#fffceab0','#fff3bf55'][i];c.stroke();}
 c.save();shape(12);c.clip();
 // Fixed facets keep the surface tactile without turning it into a noisy pixel grid.
 for(let i=0;i<105;i++){
  const angle=random(i+4)*tau,rad=72+random(i+91)*38,x=Math.cos(angle)*rad,y=Math.sin(angle)*rad,size=4+random(i+53)*9;
  c.fillStyle=`rgba(255,244,201,${(.025+random(i+81)*.13)*(y<0?1:.45)})`;c.fillRect(x,y,size,size);
 }
 const shine=c.createLinearGradient(-80,-95,70,90);shine.addColorStop(0,'#ffffff00');shine.addColorStop(.13,'#fff9df80');shine.addColorStop(.24,'#fff4da05');shine.addColorStop(.8,'#fff7cb00');shine.addColorStop(1,'#fff3b38c');
 c.fillStyle=shine;c.fillRect(-r,-r,r*2,r*2);c.restore();
 if(transcribing){
  const motion=phase==='canceling'?t*.3:t;
  const depth=c.createRadialGradient(-12,-15,4,0,0,58);
  depth.addColorStop(0,'#88492b60');depth.addColorStop(.8,'#a2573280');depth.addColorStop(1,'#ffd28f28');
  c.beginPath();c.arc(0,0,58,0,tau);c.fillStyle=depth;c.fill();
  c.save();c.rotate(motion*.34);
  const gold=c.createConicGradient(-motion*.12,0,0);
  ['#fff4ba','#b77539','#f7bf70','#fffbe0','#ce8a48','#ffdd91','#9e562c','#fff2b5','#fff4ba'].forEach((color,i)=>gold.addColorStop(i/8,color));
  c.beginPath();c.arc(0,0,68,0,tau);c.lineWidth=23;c.strokeStyle=gold;c.stroke();
  for(let i=0;i<48;i++){
   const a=i*tau/48,radius=68+Math.sin(motion*.7+i*.8)*1.5;
   c.beginPath();c.arc(0,0,radius,a,a+tau/48-.006);
   c.lineWidth=12+Math.sin(motion*.8+i*1.3)*7;
   c.strokeStyle=i%5===0?'#fffce9c9':i%3===0?'#9c512e55':'#ffe7a651';c.stroke();
  }
  for(const radius of [55,58,79,83]){c.beginPath();c.arc(0,0,radius,0,tau);c.lineWidth=radius===55?2:1;c.strokeStyle=radius===55?'#fff8cee6':'#ffe4a777';c.stroke();}
  c.restore();
 }
 if(recording){
  for(let i=0;i<96;i++){
   // Repeat the full frequency range in each quadrant so speech's lower
   // frequencies cannot concentrate all activity in the upper semicircle.
   const quadrant=Math.floor(i/24),position=quadrant%2===0?i%24:23-i%24;
   const index=meeting?i%2:((position*7)%24)*2;
   const energy=Math.max(0,Math.min(1,meeting?(bands[index]??0):Math.max(bands[index]??0,bands[index+1]??0)));
   const angle=i*tau/96-Math.PI/2,len=Math.min(76,4*(3+energy*35));
   // Crossfade seeded blue selections instead of flickering or moving bars.
   const cycle=t/1.2,epoch=Math.floor(cycle),blend=(1-Math.cos((cycle-epoch)*Math.PI))/2;
   const before=random(i+epoch*113)>.73?1:0,after=random(i+(epoch+1)*113)>.73?1:0;
   const blue=meeting?0:before+(after-before)*blend;
   c.save();c.rotate(angle);c.fillStyle=`hsla(${19+energy*16+Math.sin(t*1.1+i*.3)*4},95%,${62+energy*16}%,${.42+energy*.55})`;
   if(meeting)c.fillStyle=i%2===0?`rgba(211,91,170,${.42+energy*.55})`:`rgba(235,161,57,${.42+energy*.55})`;
   c.fillRect(119,-1.35,len,2.7);c.fillRect(123+len,-1,2,2);c.restore();
   if(blue>0){c.save();c.rotate(angle);c.fillStyle=`rgba(24,143,255,${blue*(.75+energy*.25)})`;c.fillRect(119,-1.35,len,2.7);c.fillRect(123+len,-1,2,2);c.restore();}
  }
 }
 if(phase!=='canceling'){
  const complete=phase==='complete',count=complete?12:26;
  for(let i=0;i<count;i++){
   const life=2.6+random(i+112)*3.4,cycle=t/life+random(i+7),epoch=Math.floor(cycle),u=reduced?random(i+17):cycle-epoch;
   const seed=i+epoch*73,base=random(seed+31)*tau;
   const orbit=transcribing;
   const angle=base+(orbit?u*(.7+random(seed+83)*1.4):0);
   const radius=complete?112+random(seed+18)*13:orbit?(i%4===0?72+random(seed+9)*20:112+random(seed+9)*26):18+u*(100+random(seed+19)*38);
   const opacity=Math.pow(Math.sin(u*Math.PI),1.8)*(complete?.95:.8);
   const size=(2+random(seed+51)*5)*(complete?1.3:1),blue=i%7===0;
   c.save();c.translate(Math.cos(angle)*radius,Math.sin(angle)*radius);
   c.rotate(orbit?base+u*(random(seed+82)>.5?1:-1)*tau:base*.15);
   c.globalAlpha=reduced?.5:opacity;c.fillStyle=blue?'#249dff':i%3===0?'#ffdc97':'#fffbea';c.shadowColor=blue?'#147dff':'#fff3c7';c.shadowBlur=blue?7:5;
   if(complete)star(c,0,0,size*(.5+Math.sin(u*Math.PI)));else c.fillRect(-size/2,-size/2,size,size);
   c.restore();
  }
 }
 c.restore();
}
