/** Serialize refreshes and coalesce bursts, preserving requests made during a read. */
export function refreshQueue(read:(full:boolean)=>Promise<void>) {
 let pending:{full:boolean;resolve:()=>void;reject:(error:unknown)=>void}[]=[];
 let running=false;
 async function drain(){
  while(pending.length){
   const batch=pending;pending=[];
   try{await read(batch.some(r=>r.full));for(const r of batch)r.resolve();}
   catch(error){for(const r of batch)r.reject(error);}
  }
  running=false;
 }
 return (full=false)=>new Promise<void>((resolve,reject)=>{
  pending.push({full,resolve,reject});
  if(!running){running=true;queueMicrotask(()=>void drain());}
 });
}
