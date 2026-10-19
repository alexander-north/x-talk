import {describe,it,expect} from 'vitest';
import {defaultColumns,restoreColumns,moveColumn} from './recordings';
describe('recordings column preferences',()=>{
 it('validates persisted data, drops unknown/duplicate columns, bounds widths, and keeps a column visible',()=>{
  const columns=restoreColumns(JSON.stringify([{key:'transcript',width:100000,visible:false},{key:'transcript',width:10,visible:true},{key:'unsafe',visible:true},...defaultColumns.filter(c=>c.key!=='transcript').map(c=>({...c,width:-1,visible:false}))]));
  expect(columns).toHaveLength(defaultColumns.length);expect(new Set(columns.map(c=>c.key)).size).toBe(defaultColumns.length);
  expect(columns[0]).toMatchObject({key:'transcript',width:700,visible:true});expect(columns[1].width).toBe(100);
  expect(restoreColumns('invalid')).toEqual(defaultColumns);expect(restoreColumns('{}')).toEqual(defaultColumns);
 });
 it('moves columns without mutating defaults and preserves boundary order',()=>{
  const result=moveColumn(defaultColumns,'transcript',-1);expect(result[0].key).toBe('transcript');expect(defaultColumns[0].key).toBe('created');expect(moveColumn(result,'transcript',-1)).toEqual(result);
 });
});

describe('recording summaries',()=>{
 it('drops legacy subtitle columns and limits previews without changing copied source text',async()=>{
  const {wordCount,preview}=await import('./recordings');
  expect(restoreColumns('[{"key":"subtitle","visible":true}]').some(c=>c.key==='subtitle')).toBe(false);
  expect(wordCount(null)).toBe(0);expect(wordCount('  one\n two   three ')).toBe(3);
  expect(preview('one\ntwo\nthree\nfour')).toBe('one\ntwo\nthree…');expect(preview('short')).toBe('short');expect(preview('a'.repeat(300))).toHaveLength(258);
 });
});
