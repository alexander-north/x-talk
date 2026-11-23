//! Core Audio meeting capture for macOS 14.2 or later.
//!
//! - Microphone: an IO proc on the selected (or default) input device, read in
//!   the device's native rate and channel layout.
//! - System audio: a private Core Audio process tap of the selected (or default)
//!   output device's first stream, excluding no process (as WASAPI loopback
//!   includes every process, xTalk itself too: the app suppresses its own cues
//!   during meetings). The tap is wrapped in a private aggregate device read by
//!   an IO proc. Only the system-audio-recording permission
//!   (NSAudioCaptureUsageDescription) is involved, not screen recording.
//!
//! The IO procs copy samples into lock-free queues; the capture thread places
//! them on the shared timeline with `capture_core::TrackWriter`, exactly as the
//! WASAPI backend does with packet QPC timestamps. Core Audio host time plays
//! the role of QPC. Device loss, rate changes and permission denial end the
//! recording with an actionable error and keep the partial journal.
//!
//! Every FFI call below was checked against objc2-core-audio 0.3.2,
//! objc2-core-audio-types 0.3.2, objc2-core-foundation 0.3.2,
//! objc2-foundation 0.3.2 and objc2-avf-audio 0.3.2. Unsafe blocks are kept
//! small and state the invariant they rely on.

use super::{DeviceChange, Journal, MonitorStatus, Options, Status, Track};
use crate::domain::{AppError, Result};
use crate::meetings::capture_core::{
    self as neutral, Converter, RateCheck, SampleClock, TrackWriter,
};
use crossbeam_queue::ArrayQueue;
use objc2::AnyThread;
use objc2_avf_audio::{AVAudioApplication, AVAudioApplicationRecordPermission};
use objc2_core_audio::{
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceIsAlive, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams,
    kAudioHardwareBadPropertySizeError, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyTranslateUIDToDevice,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioStreamPropertyVirtualFormat, kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey,
    kAudioTapPropertyFormat, AudioConvertHostTimeToNanos, AudioDeviceCreateIOProcID,
    AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
    AudioGetCurrentHostTime, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyScope, AudioObjectPropertySelector,
    CATapDescription, CATapMuteBehavior,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat, kAudioFormatLinearPCM, AudioBuffer,
    AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, AudioTimeStampFlags,
};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_foundation::{NSArray, NSNumber, NSString};
use std::{
    ffi::{c_void, CStr},
    path::PathBuf,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const MICROPHONE_DENIED: &str = "xTalk does not have microphone access. Allow xTalk in System Settings > Privacy & Security > Microphone, then try again.";
const SYSTEM_SETTINGS_AUDIO_CAPTURE: &str = "System Settings > Privacy & Security > Screen & System Audio Recording (System Audio Recording Only)";

fn device_error() -> AppError {
    AppError::new(
        "meeting_capture",
        "device",
        "An audio device became unavailable. Captured audio is preserved; check the selected devices in System Settings > Sound.",
    )
}
fn core_audio_error(action: &str, status: i32) -> AppError {
    AppError::new(
        "meeting_capture",
        "device",
        format!(
            "Could not {action} (Core Audio error {}). Check the selected devices in System Settings > Sound.",
            neutral::os_status(status)
        ),
    )
}
fn system_audio_denied(status: i32) -> AppError {
    AppError::new(
        "meeting_capture",
        "permission",
        format!(
            "macOS did not allow xTalk to capture system audio (Core Audio error {}). Allow xTalk in {SYSTEM_SETTINGS_AUDIO_CAPTURE}, then try again. System audio capture requires macOS 14.2 or later.",
            neutral::os_status(status)
        ),
    )
}
fn silent_system_hint() -> String {
    format!("System audio was digital silence. If something was playing, allow xTalk in {SYSTEM_SETTINGS_AUDIO_CAPTURE} and check that the meeting app plays through the selected output device.")
}

// ---------------------------------------------------------------------------
// Property access

fn address(
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}
const SYSTEM_OBJECT: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;

/// Reads a fixed-size property.
///
/// # Safety
/// `T` must be the exact plain-data type Core Audio stores for `selector`,
/// and `qualifier`, when given, must point to `size` readable bytes of the
/// selector's qualifier type for the duration of the call.
unsafe fn read<T: Copy>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    qualifier: Option<(*const c_void, u32)>,
    initial: T,
) -> std::result::Result<T, i32> {
    let address = address(selector, scope);
    let mut value = initial;
    let mut size = std::mem::size_of::<T>() as u32;
    let (qualifier, qualifier_size) = qualifier.unwrap_or((std::ptr::null(), 0));
    // SAFETY: `value` is a valid, writable `T` of `size` bytes and the caller
    // guarantees `T` and the qualifier match the selector.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            qualifier_size,
            qualifier,
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    if status != 0 {
        return Err(status);
    }
    if size as usize != std::mem::size_of::<T>() {
        return Err(kAudioHardwareBadPropertySizeError);
    }
    Ok(value)
}

/// Reads an array-of-`AudioObjectID` property such as a device's streams.
fn read_ids(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> std::result::Result<Vec<AudioObjectID>, i32> {
    let address = address(selector, scope);
    let mut size = 0u32;
    // SAFETY: only writes the byte size of the property into `size`.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 {
        return Err(status);
    }
    let count = size as usize / std::mem::size_of::<AudioObjectID>();
    let mut ids: Vec<AudioObjectID> = vec![kAudioObjectUnknown; count];
    if count == 0 {
        return Ok(ids);
    }
    let mut size = (count * std::mem::size_of::<AudioObjectID>()) as u32;
    // SAFETY: `ids` provides `size` writable bytes of AudioObjectIDs, the type
    // Core Audio stores for object-list properties.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(ids.as_mut_ptr())
                .expect("non-empty vector")
                .cast(),
        )
    };
    if status != 0 {
        return Err(status);
    }
    ids.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
    Ok(ids)
}

fn default_device(input: bool) -> Result<AudioObjectID> {
    let selector = if input {
        kAudioHardwarePropertyDefaultInputDevice
    } else {
        kAudioHardwarePropertyDefaultOutputDevice
    };
    // SAFETY: the default-device properties hold an AudioObjectID.
    let device = unsafe {
        read(
            SYSTEM_OBJECT,
            selector,
            kAudioObjectPropertyScopeGlobal,
            None,
            kAudioObjectUnknown,
        )
    }
    .map_err(|status| core_audio_error("find the default audio device", status))?;
    if device == kAudioObjectUnknown {
        return Err(AppError::input(if input {
            "No microphone is available. Connect one or choose an input in System Settings > Sound."
        } else {
            "No output device is available. Connect one or choose an output in System Settings > Sound."
        }));
    }
    Ok(device)
}

fn device_for_uid(uid: &str, input: bool) -> Result<AudioObjectID> {
    let uid = CFString::from_str(uid);
    let reference: *const CFString = &*uid;
    // SAFETY: TranslateUIDToDevice takes a CFStringRef qualifier (`reference`,
    // kept alive by `uid`) and stores an AudioObjectID.
    let device = unsafe {
        read(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyTranslateUIDToDevice,
            kAudioObjectPropertyScopeGlobal,
            Some((
                (&reference as *const *const CFString).cast(),
                std::mem::size_of::<*const CFString>() as u32,
            )),
            kAudioObjectUnknown,
        )
    }
    .map_err(|status| core_audio_error("look up the selected audio device", status))?;
    if device == kAudioObjectUnknown {
        return Err(AppError::input(if input {
            "The selected microphone is not connected. Choose another microphone."
        } else {
            "The selected output device is not connected. Choose another output device."
        }));
    }
    Ok(device)
}

fn device_uid(device: AudioObjectID) -> Result<String> {
    // SAFETY: DeviceUID stores a CFStringRef that the caller owns (+1).
    let uid = unsafe {
        read::<*const CFString>(
            device,
            kAudioDevicePropertyDeviceUID,
            kAudioObjectPropertyScopeGlobal,
            None,
            std::ptr::null(),
        )
    }
    .map_err(|status| core_audio_error("identify the output device", status))?;
    let uid = NonNull::new(uid.cast_mut()).ok_or_else(device_error)?;
    // SAFETY: the property follows the Create rule; CFRetained releases it.
    let uid = unsafe { CFRetained::from_raw(uid) };
    Ok(uid.to_string())
}

fn alive(device: AudioObjectID) -> bool {
    // SAFETY: DeviceIsAlive stores a UInt32.
    unsafe {
        read(
            device,
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
            None,
            0u32,
        )
    }
    .is_ok_and(|alive| alive != 0)
}

fn nominal_rate(device: AudioObjectID) -> std::result::Result<f64, i32> {
    // SAFETY: NominalSampleRate stores a Float64.
    unsafe {
        read(
            device,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
            None,
            0f64,
        )
    }
}

const EMPTY_FORMAT: AudioStreamBasicDescription = AudioStreamBasicDescription {
    mSampleRate: 0.0,
    mFormatID: 0,
    mFormatFlags: 0,
    mBytesPerPacket: 0,
    mFramesPerPacket: 0,
    mBytesPerFrame: 0,
    mChannelsPerFrame: 0,
    mBitsPerChannel: 0,
    mReserved: 0,
};
/// IO procs receive each stream in its virtual format; on macOS that is
/// native-endian 32-bit float PCM for mixable streams. Anything else is
/// refused rather than misread.
fn float_format(format: &AudioStreamBasicDescription) -> Result<()> {
    if format.mFormatID == kAudioFormatLinearPCM
        && format.mFormatFlags & kAudioFormatFlagIsFloat != 0
        && format.mFormatFlags & kAudioFormatFlagIsBigEndian == 0
        && format.mBitsPerChannel == 32
    {
        Ok(())
    } else {
        Err(AppError::input(
            "The selected audio device does not provide 32-bit float audio to applications. Choose another device.",
        ))
    }
}

/// Native rate and total channel count of a device's input streams.
fn input_format(device: AudioObjectID) -> Result<(f64, u32)> {
    let streams = read_ids(
        device,
        kAudioDevicePropertyStreams,
        kAudioObjectPropertyScopeInput,
    )
    .map_err(|status| core_audio_error("read the microphone format", status))?;
    let mut rate = 0.0;
    let mut channels = 0;
    for stream in streams {
        // SAFETY: VirtualFormat stores an AudioStreamBasicDescription.
        let format = unsafe {
            read(
                stream,
                kAudioStreamPropertyVirtualFormat,
                kAudioObjectPropertyScopeGlobal,
                None,
                EMPTY_FORMAT,
            )
        }
        .map_err(|status| core_audio_error("read the microphone format", status))?;
        float_format(&format)?;
        if rate == 0.0 {
            rate = format.mSampleRate;
        }
        channels += format.mChannelsPerFrame;
    }
    if channels == 0 {
        return Err(AppError::input(
            "The selected microphone has no input channels. Choose another microphone.",
        ));
    }
    Ok((rate, channels))
}

fn microphone_permission() -> Result<()> {
    // Capture threads have no autorelease pool of their own.
    let permission = objc2::rc::autoreleasepool(|_| {
        // SAFETY: AVAudioApplication (macOS 14+) is a thread-safe singleton
        // and recordPermission is a plain getter.
        unsafe { AVAudioApplication::sharedInstance().recordPermission() }
    });
    if permission == AVAudioApplicationRecordPermission::Denied {
        return Err(AppError::new(
            "meeting_capture",
            "permission",
            MICROPHONE_DENIED,
        ));
    }
    Ok(())
}

/// Nanoseconds on the Core Audio host clock (mach absolute time).
fn host_nanos(host_time: u64) -> u64 {
    // SAFETY: pure conversion with no preconditions.
    unsafe { AudioConvertHostTimeToNanos(host_time) }
}
fn now_nanos() -> u64 {
    // SAFETY: reads the host clock; no preconditions.
    host_nanos(unsafe { AudioGetCurrentHostTime() })
}

// ---------------------------------------------------------------------------
// Real-time IO: lock-free hand-off from the Core Audio IO thread

#[derive(Clone, Copy)]
struct Packet {
    host_time: Option<u64>,
    sample_time: Option<f64>,
    frames: u32,
}
struct Shared {
    /// Journal channel count (1 or 2): the first channels of the device.
    keep: usize,
    samples: ArrayQueue<f32>,
    packets: ArrayQueue<Packet>,
    overruns: AtomicU64,
    callbacks: AtomicU64,
    missing_channels: AtomicBool,
}
impl Shared {
    fn new(keep: usize, rate: f64) -> Self {
        // Two seconds of headroom; the capture thread drains every 5 ms.
        let frames = (rate.max(8000.0) as usize) * 2;
        Self {
            keep,
            samples: ArrayQueue::new(frames * keep),
            packets: ArrayQueue::new(8192),
            overruns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            missing_channels: AtomicBool::new(false),
        }
    }
    /// Runs on the Core Audio IO thread: no locks, allocation or blocking.
    /// A packet that does not fit is dropped whole and counted, so sample and
    /// packet queues always stay consistent.
    fn capture(&self, buffers: &[AudioBuffer], time: &AudioTimeStamp) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        // (buffer data, channel offset, interleave stride) of each kept channel.
        let mut picks = [(std::ptr::null::<f32>(), 0usize, 1usize); 2];
        let mut found = 0;
        let mut frames = usize::MAX;
        for buffer in buffers {
            let channels = buffer.mNumberChannels as usize;
            if buffer.mData.is_null() || channels == 0 {
                continue;
            }
            frames = frames.min(buffer.mDataByteSize as usize / (4 * channels));
            for channel in 0..channels {
                if found < self.keep {
                    picks[found] = (buffer.mData as *const f32, channel, channels);
                    found += 1;
                }
            }
        }
        if found == 0 {
            // No input data this cycle; timeline gaps are filled from host time.
            return;
        }
        if found < self.keep {
            self.missing_channels.store(true, Ordering::Release);
            return;
        }
        if frames == 0 || frames == usize::MAX || frames > u32::MAX as usize {
            return;
        }
        let free = self.samples.capacity() - self.samples.len();
        if free < frames * self.keep || self.packets.is_full() {
            self.overruns.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for frame in 0..frames {
            for &(data, channel, stride) in &picks[..self.keep] {
                // SAFETY: `frame < frames <= mDataByteSize / (4 * stride)` and
                // `channel < stride`, so the read stays inside the IO buffer,
                // which Core Audio keeps valid for this callback.
                let value = unsafe { data.add(frame * stride + channel).read() };
                // Cannot fail: free space was checked and this is the only producer.
                let _ = self.samples.push(value);
            }
        }
        let flags = time.mFlags.0;
        let _ = self.packets.push(Packet {
            host_time: (flags & AudioTimeStampFlags::HostTimeValid.0 != 0)
                .then_some(time.mHostTime),
            sample_time: (flags & AudioTimeStampFlags::SampleTimeValid.0 != 0)
                .then_some(time.mSampleTime),
            frames: frames as u32,
        });
    }
}

/// The AudioDeviceIOProc registered for every capture device.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    // Never unwind into Core Audio.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: `client` is the `Shared` registered in `HalInput::open`; the
        // HalInput owns an Arc to it until after AudioDeviceDestroyIOProcID.
        let shared = unsafe { &*(client as *const Shared) };
        // SAFETY: Core Audio passes a valid buffer list whose `mBuffers` array
        // holds `mNumberBuffers` entries, and a valid input time stamp, both
        // valid for the duration of this call.
        let (buffers, time) = unsafe {
            let list = input.as_ptr();
            let count = (*list).mNumberBuffers as usize;
            let first = std::ptr::addr_of!((*list).mBuffers).cast::<AudioBuffer>();
            (
                std::slice::from_raw_parts(first, count),
                input_time.as_ref(),
            )
        };
        shared.capture(buffers, time);
    }));
    0
}

/// An IO proc registered on a device. Dropping it stops and unregisters the
/// proc before the shared queues can be freed.
struct HalInput {
    device: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    running: bool,
    shared: Arc<Shared>,
}
impl HalInput {
    fn open(device: AudioObjectID, shared: Arc<Shared>) -> Result<Self> {
        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: `io_proc` matches AudioDeviceIOProc; the client pointer stays
        // valid because `shared` lives in the returned HalInput, which destroys
        // the proc in Drop before releasing it.
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                device,
                Some(io_proc),
                Arc::as_ptr(&shared).cast_mut().cast(),
                NonNull::from(&mut proc_id),
            )
        };
        if status != 0 || proc_id.is_none() {
            return Err(core_audio_error("open the audio device", status));
        }
        Ok(Self {
            device,
            proc_id,
            running: false,
            shared,
        })
    }
    fn start(&mut self) -> Result<()> {
        // SAFETY: `proc_id` was created for `device` and not yet destroyed.
        let status = unsafe { AudioDeviceStart(self.device, self.proc_id) };
        if status != 0 {
            return Err(core_audio_error("start the audio device", status));
        }
        self.running = true;
        Ok(())
    }
    fn stop(&mut self) {
        if self.running {
            // SAFETY: as in `start`. Called outside the IO thread, it returns
            // once the proc is no longer running.
            let _ = unsafe { AudioDeviceStop(self.device, self.proc_id) };
            self.running = false;
        }
    }
}
impl Drop for HalInput {
    fn drop(&mut self) {
        self.stop();
        // SAFETY: the proc is stopped and destroyed exactly once.
        let _ = unsafe { AudioDeviceDestroyIOProcID(self.device, self.proc_id) };
    }
}

/// A private process tap of one output device plus the private aggregate
/// device that exposes it as input. Dropped in reverse creation order; any
/// leak would otherwise persist until xTalk exits.
struct SystemTap {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    format: AudioStreamBasicDescription,
}
impl SystemTap {
    fn create(output_uid: &str) -> Result<Self> {
        // Capture threads have no autorelease pool of their own.
        objc2::rc::autoreleasepool(|_| Self::create_in_pool(output_uid))
    }
    fn create_in_pool(output_uid: &str) -> Result<Self> {
        let excluded: objc2::rc::Retained<NSArray<NSNumber>> = NSArray::new();
        let uid = NSString::from_str(output_uid);
        // SAFETY: standard alloc/init of CATapDescription (macOS 14.2+) with
        // valid NSArray/NSString arguments; stream 0 is the device's first
        // output stream. Excluding no process captures everything it plays.
        let description = unsafe {
            CATapDescription::initExcludingProcesses_andDeviceUID_withStream(
                CATapDescription::alloc(),
                &excluded,
                &uid,
                0,
            )
        };
        // SAFETY: plain property setters on a fully initialized description.
        unsafe {
            description.setName(&NSString::from_str("xTalk meeting system audio"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }
        let mut tap = SystemTap {
            tap: kAudioObjectUnknown,
            aggregate: kAudioObjectUnknown,
            format: EMPTY_FORMAT,
        };
        // SAFETY: `description` is valid and `tap.tap` is a writable AudioObjectID.
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap.tap) };
        if status != 0 || tap.tap == kAudioObjectUnknown {
            tap.tap = kAudioObjectUnknown;
            return Err(system_audio_denied(status));
        }
        // SAFETY: TapFormat stores an AudioStreamBasicDescription.
        tap.format = unsafe {
            read(
                tap.tap,
                kAudioTapPropertyFormat,
                kAudioObjectPropertyScopeGlobal,
                None,
                EMPTY_FORMAT,
            )
        }
        .map_err(|status| core_audio_error("read the system audio format", status))?;
        float_format(&tap.format)?;
        // SAFETY: plain getters on the initialized description.
        let tap_uid = unsafe { description.UUID().UUIDString() }.to_string();
        let description = aggregate_description(&tap_uid);
        let description: &CFDictionary =
            <CFDictionary<CFString, CFType> as AsRef<CFDictionary>>::as_ref(&description);
        // SAFETY: the dictionary holds only the documented aggregate keys with
        // CFString/CFBoolean/CFArray values; `tap.aggregate` is writable.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(description, NonNull::from(&mut tap.aggregate))
        };
        if status != 0 || tap.aggregate == kAudioObjectUnknown {
            tap.aggregate = kAudioObjectUnknown;
            return Err(core_audio_error("create the system audio device", status));
        }
        Ok(tap)
    }
}
impl Drop for SystemTap {
    fn drop(&mut self) {
        // SAFETY: each object was created by this value and is destroyed once.
        unsafe {
            if self.aggregate != kAudioObjectUnknown {
                let _ = AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            if self.tap != kAudioObjectUnknown {
                let _ = AudioHardwareDestroyProcessTap(self.tap);
            }
        }
    }
}

fn key(name: &CStr) -> CFRetained<CFString> {
    CFString::from_str(name.to_str().unwrap_or_default())
}
/// The aggregate holds only the tap (as in cpal 0.17's loopback device): it
/// is private to this process, drift-compensated, and never auto-started.
fn aggregate_description(tap_uid: &str) -> CFRetained<CFDictionary<CFString, CFType>> {
    let tap_uid = CFString::from_str(tap_uid);
    let sub_tap_keys = [
        key(kAudioSubTapUIDKey),
        key(kAudioSubTapDriftCompensationKey),
    ];
    let sub_tap_keys: [&CFString; 2] = [&sub_tap_keys[0], &sub_tap_keys[1]];
    let sub_tap_values: [&CFType; 2] = [&tap_uid, CFBoolean::new(true)];
    let sub_tap = CFDictionary::<CFString, CFType>::from_slices(&sub_tap_keys, &sub_tap_values);
    let sub_tap: &CFType = &sub_tap;
    let taps = CFArray::<CFType>::from_objects(&[sub_tap]);
    let name = CFString::from_str("xTalk meeting system audio");
    let uid = CFString::from_str(&format!(
        "{}{}",
        neutral::AGGREGATE_UID_PREFIX,
        uuid::Uuid::new_v4()
    ));
    let keys = [
        key(kAudioAggregateDeviceNameKey),
        key(kAudioAggregateDeviceUIDKey),
        key(kAudioAggregateDeviceIsPrivateKey),
        key(kAudioAggregateDeviceTapAutoStartKey),
        key(kAudioAggregateDeviceTapListKey),
    ];
    let keys: [&CFString; 5] = [&keys[0], &keys[1], &keys[2], &keys[3], &keys[4]];
    let values: [&CFType; 5] = [
        &name,
        &uid,
        CFBoolean::new(true),
        CFBoolean::new(false),
        &taps,
    ];
    CFDictionary::<CFString, CFType>::from_slices(&keys, &values)
}

// ---------------------------------------------------------------------------
// Sources

/// One running capture device (microphone or system tap).
struct Source {
    // Field order is drop order: stop the IO proc, then destroy the tap.
    io: HalInput,
    _tap: Option<SystemTap>,
    track: Track,
    microphone: bool,
    /// The physical device whose liveness and rate are monitored.
    watched: AudioObjectID,
    nominal: Option<f64>,
    converter: Converter,
    clock: SampleClock,
    rate: RateCheck,
    raw: Vec<f32>,
    converted: Vec<f32>,
    callbacks: u64,
    progress_at: f64,
    checked_at: f64,
    permission_at: f64,
}
impl Source {
    fn open(microphone: bool, selected: Option<&str>) -> Result<Self> {
        let device = match neutral::coreaudio_uid(selected) {
            Some(uid) => device_for_uid(uid, microphone)?,
            None => default_device(microphone)?,
        };
        let (capture_device, tap, rate, channels) = if microphone {
            let (rate, channels) = input_format(device)?;
            microphone_permission()?;
            (device, None, rate, channels)
        } else {
            let tap = SystemTap::create(&device_uid(device)?)?;
            let (rate, channels) = (tap.format.mSampleRate, tap.format.mChannelsPerFrame);
            (tap.aggregate, Some(tap), rate, channels)
        };
        let format = neutral::plan_format(rate, channels)?;
        let shared = Arc::new(Shared::new(format.channels as usize, rate));
        let io = HalInput::open(capture_device, shared)?;
        Ok(Self {
            io,
            _tap: tap,
            track: Track {
                filename: if microphone {
                    "microphone.f32"
                } else {
                    "system.f32"
                }
                .into(),
                rate: format.rate,
                channels: format.channels,
                microphone,
            },
            microphone,
            watched: device,
            nominal: nominal_rate(device).ok(),
            converter: Converter::new(format),
            clock: SampleClock::default(),
            rate: RateCheck::new(rate),
            raw: Vec::new(),
            converted: Vec::new(),
            callbacks: 0,
            progress_at: 0.0,
            checked_at: 0.0,
            permission_at: 0.0,
        })
    }
    fn start(&mut self, elapsed: f64) -> Result<()> {
        self.progress_at = elapsed;
        self.checked_at = elapsed;
        self.io.start()
    }
    fn drain(
        &mut self,
        writer: &mut TrackWriter,
        epoch: u64,
        muted: bool,
        elapsed: f64,
    ) -> Result<f32> {
        writer.begin(muted, elapsed);
        let shared = self.io.shared.clone();
        if shared.missing_channels.load(Ordering::Acquire) {
            return Err(AppError::new(
                "meeting_capture",
                "device",
                "An audio device changed its channel layout during recording. The partial recording is preserved.",
            ));
        }
        let lost = shared.overruns.swap(0, Ordering::Relaxed);
        if lost > 0 {
            writer.discontinuities += lost;
            self.clock.reset();
            self.rate.reset();
        }
        // Bound each pass so one device cannot starve the other.
        for _ in 0..1000 {
            let Some(packet) = shared.packets.pop() else {
                break;
            };
            self.raw.clear();
            for _ in 0..packet.frames as usize * shared.keep {
                // The producer pushes samples before their packet header.
                self.raw.push(shared.samples.pop().unwrap_or(0.0));
            }
            let host_time = packet.host_time.ok_or_else(|| {
                AppError::input(
                    "An audio device reported invalid timing. The partial recording is preserved.",
                )
            })?;
            let start = (host_nanos(host_time) as i128 - epoch as i128) as f64 / 1e9;
            if self.clock.observe(packet.sample_time, packet.frames) {
                writer.discontinuities += 1;
                self.rate.reset();
            }
            self.rate.observe(start, packet.frames)?;
            self.converter.convert(&self.raw, &mut self.converted);
            writer.packet(start, &self.converted, muted, elapsed)?;
        }
        writer.end(muted, elapsed)
    }
    /// Device loss, rate changes and revoked permission end the recording.
    fn health(&mut self, elapsed: f64) -> Result<()> {
        if elapsed - self.checked_at < 0.25 {
            return Ok(());
        }
        self.checked_at = elapsed;
        if !alive(self.watched) {
            return Err(device_error());
        }
        if let (Some(expected), Ok(rate)) = (self.nominal, nominal_rate(self.watched)) {
            if (rate - expected).abs() > 0.5 {
                return Err(AppError::new(
                    "meeting_capture",
                    "device",
                    format!("An audio device changed its sample rate from {expected:.0} Hz to {rate:.0} Hz during recording. The partial recording is preserved; keep the rate fixed in Audio MIDI Setup while recording."),
                ));
            }
        }
        if self.microphone {
            if elapsed - self.permission_at >= 1.0 {
                self.permission_at = elapsed;
                microphone_permission()?;
            }
            // A running input device delivers continuously; a render tap may
            // legitimately pause while nothing plays, so it is not watched.
            let callbacks = self.io.shared.callbacks.load(Ordering::Relaxed);
            if callbacks != self.callbacks {
                self.callbacks = callbacks;
                self.progress_at = elapsed;
            } else if callbacks == 0 && elapsed - self.progress_at > 15.0 {
                return Err(AppError::new(
                    "meeting_capture",
                    "permission",
                    format!("The microphone did not deliver any audio. {MICROPHONE_DENIED}"),
                ));
            } else if callbacks > 0 && elapsed - self.progress_at > 5.0 {
                return Err(device_error());
            }
        }
        Ok(())
    }
    fn stop(&mut self) {
        self.io.stop();
    }
}

struct Input {
    source: Source,
    writer: TrackWriter,
}
impl Input {
    fn open(
        directory: Option<&std::path::Path>,
        microphone: bool,
        selected: Option<&str>,
    ) -> Result<Self> {
        let source = Source::open(microphone, selected)?;
        let writer = TrackWriter::create(directory, source.track.clone(), !microphone)?;
        Ok(Self { source, writer })
    }
    fn drain(&mut self, epoch: u64, muted: bool, elapsed: f64) -> Result<f32> {
        self.source.drain(&mut self.writer, epoch, muted, elapsed)
    }
}

fn open_inputs(options: &Options, directory: Option<&std::path::Path>) -> Result<Vec<Input>> {
    let mut inputs = Vec::new();
    if options.microphone {
        inputs.push(Input::open(
            directory,
            true,
            options.microphone_device.as_deref(),
        )?);
    }
    if options.system {
        inputs.push(Input::open(
            directory,
            false,
            options.output_device.as_deref(),
        )?);
    }
    Ok(inputs)
}

fn silent_system(inputs: &[Input], system_muted: bool) -> bool {
    !system_muted
        && inputs
            .iter()
            .any(|input| !input.writer.track.microphone && input.writer.silent())
}

pub fn monitor(
    options: Options,
    status: Arc<Mutex<MonitorStatus>>,
    stop: Arc<AtomicBool>,
    renewed: Arc<Mutex<Instant>>,
) -> Result<()> {
    let mut inputs = open_inputs(&options, None)?;
    let epoch = now_nanos();
    let clock = Instant::now();
    for input in &mut inputs {
        input.source.start(0.0)?;
    }
    while !stop.load(Ordering::Acquire)
        && renewed
            .lock()
            .is_ok_and(|r| r.elapsed() < Duration::from_secs(2))
    {
        let mut levels = MonitorStatus {
            active: true,
            ..MonitorStatus::default()
        };
        for input in &mut inputs {
            let elapsed = clock.elapsed().as_secs_f64();
            input.source.health(elapsed)?;
            let level = input.drain(epoch, false, elapsed)?;
            if input.writer.track.microphone {
                levels.microphone_level = level;
            } else {
                levels.system_level = level;
            }
        }
        levels.spectrum =
            crate::spectrum::combine(inputs.iter().map(|input| &input.writer.spectrum)).to_vec();
        if let Ok(mut state) = status.lock() {
            *state = levels;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn record(
    id: String,
    directory: PathBuf,
    options: Options,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    mic_muted: Arc<AtomicBool>,
    system_muted: Arc<AtomicBool>,
    ready: &std::sync::mpsc::SyncSender<Result<()>>,
    changes: std::sync::mpsc::Receiver<DeviceChange>,
) -> Result<Journal> {
    std::fs::create_dir_all(&directory)?;
    // Fail before capture when there is not even one minute of working headroom.
    if fs2::available_space(&directory)? < 128 * 1024 * 1024 {
        return Err(AppError::input(
            "Free some disk space before recording a meeting.",
        ));
    }
    let mut inputs = open_inputs(&options, Some(&directory))?;
    let muted = |microphone: bool| {
        if microphone {
            mic_muted.load(Ordering::Acquire)
        } else {
            system_muted.load(Ordering::Acquire)
        }
    };
    let mut journal = Journal {
        id,
        created: crate::domain::now(),
        tracks: inputs.iter().map(|i| i.writer.track.clone()).collect(),
        elapsed: 0.0,
        ready: false,
        discarded: false,
        warning: None,
    };
    let journal_path = directory.join("capture.json");
    crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?)?;
    let epoch = now_nanos();
    let clock = Instant::now();
    for input in &mut inputs {
        input.source.start(0.0)?;
    }
    if stop.load(Ordering::Acquire) {
        // The start handshake timed out (for example while a permission prompt
        // was open). Nothing was admitted; do not leave an orphaned capture.
        drop(inputs);
        let _ = std::fs::remove_dir_all(&directory);
        return Err(AppError::input(super::NOT_READY));
    }
    journal.ready = true;
    crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?)?;
    if let Ok(mut state) = status.lock() {
        state.active = true;
    }
    let _ = ready.send(Ok(()));
    let mut last_sync = Instant::now();
    let mut failure = None;
    while !stop.load(Ordering::Acquire)
        && clock.elapsed().as_secs_f64() < crate::meetings::domain::MAX_SECONDS
    {
        while let Ok(change) = changes.try_recv() {
            let result = (|| -> Result<()> {
                let old = inputs
                    .iter_mut()
                    .find(|input| input.writer.track.microphone == change.microphone)
                    .ok_or_else(|| {
                        AppError::input("This source was not enabled when recording started.")
                    })?;
                // The writer stays with the track: a failed change cannot
                // truncate or reformat the saved audio.
                let mut next = Source::open(change.microphone, change.device.as_deref())?;
                super::compatible_format(&old.writer.track, &next.track)?;
                let elapsed = clock.elapsed().as_secs_f64();
                next.start(elapsed)?;
                old.source
                    .drain(&mut old.writer, epoch, muted(change.microphone), elapsed)?;
                // Host-time packet stamps keep the common timeline across the
                // switch: the gap becomes silence and overlap is skipped.
                old.source = next;
                Ok(())
            })();
            let _ = change.reply.send(result);
        }
        let result: Result<()> = (|| {
            for input in &mut inputs {
                let elapsed = clock.elapsed().as_secs_f64();
                input.source.health(elapsed)?;
                let level = input.drain(epoch, muted(input.writer.track.microphone), elapsed)?;
                if let Ok(mut state) = status.lock() {
                    state.elapsed = clock.elapsed().as_secs_f64();
                    if input.writer.track.microphone {
                        state.microphone_level = level;
                    } else {
                        state.system_level = level;
                    }
                }
            }
            let glitches: u64 = inputs
                .iter()
                .map(|input| input.writer.discontinuities)
                .sum();
            journal.warning = neutral::capture_warning(
                "macOS",
                glitches,
                silent_system(&inputs, muted(false)),
                &silent_system_hint(),
            );
            if let Ok(mut state) = status.lock() {
                state.spectrum =
                    crate::spectrum::combine(inputs.iter().map(|input| &input.writer.spectrum))
                        .to_vec();
                state.warning = journal.warning.clone();
            }
            if last_sync.elapsed() >= Duration::from_secs(1) {
                for input in &mut inputs {
                    input.writer.sync()?;
                }
                journal.elapsed = clock.elapsed().as_secs_f64();
                crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?)?;
                if fs2::available_space(&directory)? < 32 * 1024 * 1024 {
                    return Err(AppError::input(
                        "Disk space is low. The partial meeting has been saved.",
                    ));
                }
                last_sync = Instant::now();
            }
            Ok(())
        })();
        if let Err(error) = result {
            failure = Some(error);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    journal.elapsed = clock
        .elapsed()
        .as_secs_f64()
        .min(crate::meetings::domain::MAX_SECONDS);
    for input in &mut inputs {
        if failure.is_none() {
            let microphone = input.writer.track.microphone;
            if let Err(error) = input.drain(epoch, muted(microphone), clock.elapsed().as_secs_f64())
            {
                failure = Some(error);
            }
        }
        input.source.stop();
        let keep_timeline = failure.is_none();
        if let Err(error) = input
            .writer
            .finish(journal.elapsed, &directory, keep_timeline)
        {
            failure = Some(error);
        }
    }
    // Release devices (IO procs, taps, aggregate devices) before publishing.
    drop(inputs);
    let _ = crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?);
    if let Ok(mut state) = status.lock() {
        state.capture_ended(journal.elapsed, failure.as_ref());
    }
    // Journal remains recoverable even if final disk sync failed.
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(journal)
}
