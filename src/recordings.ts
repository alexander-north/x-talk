export interface Column {key:string;label:string;width:number;visible:boolean;sortable:boolean}
export const defaultColumns:Column[]=[
 {key:'created',label:'Recording',width:180,visible:true,sortable:true},
 {key:'transcript',label:'Transcript',width:220,visible:true,sortable:true},
 {key:'transformed',label:'Transformed',width:220,visible:true,sortable:true},
 {key:'audio',label:'Audio',width:180,visible:true,sortable:false},
 {key:'source',label:'Source',width:110,visible:false,sortable:true},
 {key:'duration',label:'Duration',width:110,visible:false,sortable:true},
 {key:'bytes',label:'Audio size',width:120,visible:false,sortable:true},
 {key:'transcript_model',label:'Transcription model',width:200,visible:false,sortable:true},
 {key:'transform_model',label:'Transformation model',width:200,visible:false,sortable:true},
 {key:'id',label:'Recording ID',width:280,visible:false,sortable:true},
];
export function restoreColumns(raw:string|null):Column[]{
 try {
  const saved:unknown=JSON.parse(raw??'null');
  if(!Array.isArray(saved))return defaultColumns.map(c=>({...c}));
  const keys=new Set<string>();const result:Column[]=[];
  for(const item of saved){
   const base=defaultColumns.find(c=>c.key===item?.key);
   if(!base||keys.has(base.key))continue;
   keys.add(base.key);result.push({...base,visible:typeof item.visible==='boolean'?item.visible:base.visible,width:typeof item.width==='number'&&Number.isFinite(item.width)?Math.max(100,Math.min(700,item.width)):base.width});
  }
  for(const c of defaultColumns)if(!keys.has(c.key))result.push({...c});
  if(!result.some(c=>c.visible))result[0].visible=true;
  return result;
 }catch{return defaultColumns.map(c=>({...c}));}
}
export function moveColumn(columns:Column[],key:string,direction:number):Column[]{
 const result=columns.map(c=>({...c}));const index=result.findIndex(c=>c.key===key);const next=index+direction;
 if(index>=0&&next>=0&&next<result.length)[result[index],result[next]]=[result[next],result[index]];
 return result;
}

export function wordCount(text:string|null){return text?.trim().split(/\s+/u).filter(Boolean).length??0;}
export function preview(text:string){const lines=text.split(/\r?\n/u);const short=lines.slice(0,3).join("\n");return short.length>260?short.slice(0,257).trimEnd()+"…":lines.length>3?short+"…":short;}
