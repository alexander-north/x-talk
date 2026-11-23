import {describe,it,expect} from 'vitest';
import {detectPlatform} from './platform';
describe('platform detection',()=>{
 it('recognizes the desktop webviews',()=>{
  expect(detectPlatform({userAgent:'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)',platform:'MacIntel'})).toBe('macos');
  expect(detectPlatform({userAgent:'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0',platform:'Win32'})).toBe('windows');
  expect(detectPlatform({userAgentData:{platform:'macOS'}})).toBe('macos');
 });
 it('does not mistake other systems for a Mac or Windows',()=>{
  expect(detectPlatform({userAgent:'Mozilla/5.0 (X11; Linux x86_64)',platform:'Linux x86_64'})).toBe('other');
  expect(detectPlatform({platform:'Darwin'})).toBe('other');
  expect(detectPlatform({userAgent:'Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)'})).toBe('other');
  expect(detectPlatform(undefined)).toBe('other');
 });
});
