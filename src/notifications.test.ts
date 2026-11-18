import {afterEach,expect,it,vi} from 'vitest';
import {get} from 'svelte/store';
import {notification,notify,dismissNotification} from './notifications';
afterEach(()=>{dismissNotification();vi.useRealTimers();});
it('expires action errors and replaces the previous timer when feedback changes',()=>{
 vi.useFakeTimers();notify('Stop recording before playing meeting audio.',true);
 vi.advanceTimersByTime(6000);expect(get(notification)?.error).toBe(true);
 notify('Saved');vi.advanceTimersByTime(1000);expect(get(notification)?.text).toBe('Saved');
 vi.advanceTimersByTime(3000);expect(get(notification)).toBeNull();
});
it('allows immediate dismissal',()=>{notify('Saved');dismissNotification();expect(get(notification)).toBeNull();});
it('clears an unattended error after seven seconds',()=>{
 vi.useFakeTimers();notify('Playback unavailable',true);
 vi.advanceTimersByTime(7000);expect(get(notification)).toBeNull();
});
