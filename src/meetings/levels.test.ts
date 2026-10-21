import {describe,it,expect} from 'vitest';
import {peakDb,levelLabel} from './levels';
describe('peak meter display',()=>{
 it('shows useful quiet levels on a decibel scale without implying gain',()=>{
  expect(peakDb(1)).toBe(0);expect(peakDb(.1)).toBe(-20);expect(peakDb(.01)).toBe(-40);
  expect(peakDb(0)).toBe(-60);expect(peakDb(NaN)).toBe(-60);expect(peakDb(2)).toBe(0);
  expect(levelLabel(.1)).toBe('-20 dBFS');expect(levelLabel(0)).toBe('-60 dBFS');
 });
});
