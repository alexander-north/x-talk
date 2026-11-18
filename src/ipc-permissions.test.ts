import {readFileSync} from 'node:fs';
import {describe,it,expect} from 'vitest';
const read=(path:string)=>readFileSync(new URL(path,import.meta.url),'utf8');
describe('bundled UI native permissions',()=>{
 it('allows every frontend IPC command through the main window capability',()=>{
  const ipc=read('./ipc.ts')+read('./meetings/ipc.ts');
  const commands=[...ipc.matchAll(/invoke(?:<[^\n]*?>)?\('([^']+)'/g)].map(match=>match[1]);
  expect(commands).toContain('shortcut_editing');
  const permission=read('../src-tauri/permissions/application.toml');
  const allowed=JSON.parse(permission.match(/commands\.allow\s*=\s*(\[[\s\S]*?\])/)![1]) as string[];
  const capability=JSON.parse(read('../src-tauri/capabilities/main.json'));
  expect(capability.windows).toContain('main');
  expect(capability.permissions).toContain('allow-application');
  expect(commands.filter(command=>!allowed.includes(command))).toEqual([]);
 });
});
