import type {Settings,Transformation,Model} from './types';
export const providerNames:Record<string,string>={openai:'OpenAI',assemblyai:'AssemblyAI',elevenlabs:'ElevenLabs',deepgram:'Deepgram',anthropic:'Anthropic'};
export const settingsSections=['General','Recording','Meetings','API Keys','Providers','Transformation','Delivery and Sounds'];
export type SettingsChange=(current:Settings)=>Settings;
export function settingsWriter(load:()=>Promise<Settings>,save:(settings:Settings)=>Promise<Settings>,updated:(settings:Settings)=>Promise<void>){
 let queue:Promise<unknown>=Promise.resolve();
 return (change:SettingsChange)=>{
  const operation=queue.then(async()=>{const current=await load();const saved=await save(change(structuredClone(current)));await updated(saved);return saved;});
  queue=operation.catch(()=>undefined);return operation;
 };
}
export function nextTheme(theme:string){return ({system:'day',day:'night',night:'system'} as Record<string,string>)[theme]??'system';}
export function textDirty(draft:Transformation,base:Transformation){return ['system','user'].some(k=>draft[k as keyof Transformation]!==base[k as keyof Transformation]);}
export function thinkingLabel(level:string){return ({none:'Off · fastest',off:'Off · fastest',minimal:'Minimal · fastest',low:'Low · fast',medium:'Medium · balanced',high:'High · slower',xhigh:'Very high · slower',max:'Maximum · slowest'} as Record<string,string>)[level]??level;}
export function thinkingValue(config:Transformation,model:Model|undefined){return config.reasoning??model?.default_reasoning??'none';}
