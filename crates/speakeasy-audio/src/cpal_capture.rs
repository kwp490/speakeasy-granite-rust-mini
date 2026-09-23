use std::error::Error;
use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use speakeasy_domain::{CorrelationId, ProducerId, SessionId};

use crate::{
    AudioDiscontinuity, CallbackStamp, CaptureCallback, CaptureStreamId, NativeSampleFormat,
    NativeStreamConfig,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputDeviceDescriptor {
    pub stable_id: String,
    pub display_name: String,
    pub is_default: bool,
    pub default_config: Option<NativeStreamConfig>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureIdentity {
    pub correlation_id: CorrelationId,
    pub session_id: SessionId,
    pub producer_id: ProducerId,
    pub stream_id: CaptureStreamId,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpalCaptureRequest {
    pub identity: CaptureIdentity,
    pub device_stable_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureFault {
    pub identity: CaptureIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CpalCaptureError {
    EnumerationFailed,
    DeviceUnavailable,
    DeviceDescriptionUnavailable,
    DefaultConfigUnavailable,
    UnsupportedSampleFormat,
    StreamBuildFailed,
    StreamStartFailed,
}

impl fmt::Display for CpalCaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EnumerationFailed => "input device enumeration failed",
            Self::DeviceUnavailable => "selected input device is unavailable",
            Self::DeviceDescriptionUnavailable => "input device description is unavailable",
            Self::DefaultConfigUnavailable => "input device default format is unavailable",
            Self::UnsupportedSampleFormat => "input device sample format is unsupported",
            Self::StreamBuildFailed => "input stream construction failed",
            Self::StreamStartFailed => "input stream could not start",
        })
    }
}

impl Error for CpalCaptureError {}

/// Enumerates current input endpoints without opening a capture stream.
///
/// # Errors
///
/// Returns a sanitized adapter error when host enumeration or device
/// description retrieval fails.
pub fn enumerate_input_devices() -> Result<Vec<InputDeviceDescriptor>, CpalCaptureError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let devices = host
        .input_devices()
        .map_err(|_| CpalCaptureError::EnumerationFailed)?;
    devices
        .map(|device| describe_device(&device, default_id.as_deref()))
        .collect()
}

/// Which rule chose a [`SelectedInput`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSelectionSource {
    /// The device the caller asked for by stable id.
    Preferred,
    /// The system default input.
    SystemDefault,
    /// The first device with a usable format, because neither of the above was.
    FirstSupported,
}

/// An input device chosen in one walk and ready to start, without looking it
/// up again.
///
/// # Why this exists beside [`enumerate_input_devices`]
///
/// Starting a dictation used to walk the devices three times before the first
/// sample: once to choose one, once to read its format, and once more inside
/// [`CpalCaptureSession::start`] to find it again by id. A full walk asks every
/// device for its description and default format, and measured 17-63 ms here,
/// all of it between the key press and the microphone opening -- speech the
/// user had already begun. This asks only the devices it needs, once, and
/// hands the device itself to [`CpalCaptureSession::start_selected`].
pub struct SelectedInput {
    device: Device,
    supported: cpal::SupportedStreamConfig,
    descriptor: InputDeviceDescriptor,
    source: InputSelectionSource,
}

impl SelectedInput {
    pub const fn descriptor(&self) -> &InputDeviceDescriptor {
        &self.descriptor
    }

    pub const fn source(&self) -> InputSelectionSource {
        self.source
    }
}

/// Chooses the capture device in a single walk: `preferred_id` when it is
/// present and has a usable format, else the system default, else the first
/// device that has one.
///
/// # Errors
///
/// [`CpalCaptureError::EnumerationFailed`] when the host cannot list devices,
/// and [`CpalCaptureError::DeviceUnavailable`] when none has a usable format.
pub fn select_input_device(preferred_id: Option<&str>) -> Result<SelectedInput, CpalCaptureError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let devices: Vec<(String, Device)> = host
        .input_devices()
        .map_err(|_| CpalCaptureError::EnumerationFailed)?
        .filter_map(|device| device.id().ok().map(|id| (id.to_string(), device)))
        .collect();
    let Some((index, (supported, native), source)) =
        choose_input(&devices, preferred_id, default_id.as_deref(), |device| {
            device.default_input_config().ok().and_then(|supported| {
                native_config(&supported)
                    .ok()
                    .map(|native| (supported, native))
            })
        })
    else {
        return Err(CpalCaptureError::DeviceUnavailable);
    };
    let (id, device) = &devices[index];
    let display_name = device
        .description()
        .map_err(|_| CpalCaptureError::DeviceDescriptionUnavailable)?
        .name()
        .to_owned();
    Ok(SelectedInput {
        descriptor: InputDeviceDescriptor {
            stable_id: id.clone(),
            display_name,
            is_default: default_id.as_deref() == Some(id.as_str()),
            default_config: Some(native),
        },
        device: device.clone(),
        supported,
        source,
    })
}

/// The rule behind [`select_input_device`], apart from the hardware so it can
/// be tested: the preferred id, then the default id, then the first device, each
/// taken only if `usable` says so.
///
/// `usable` is the expensive question -- it asks the device for its format -- so
/// it is asked lazily, in that order, and never of a device after one has
/// answered. Returns the chosen index, what `usable` said about it, and why it
/// was chosen.
pub(crate) fn choose_input<D, U>(
    devices: &[(String, D)],
    preferred_id: Option<&str>,
    default_id: Option<&str>,
    mut usable: impl FnMut(&D) -> Option<U>,
) -> Option<(usize, U, InputSelectionSource)> {
    let position = |wanted: &str| devices.iter().position(|(id, _)| id == wanted);
    [
        (preferred_id, InputSelectionSource::Preferred),
        (default_id, InputSelectionSource::SystemDefault),
    ]
    .into_iter()
    .filter_map(|(wanted, source)| Some((position(wanted?)?, source)))
    .chain((0..devices.len()).map(|index| (index, InputSelectionSource::FirstSupported)))
    .find_map(|(index, source)| usable(&devices[index].1).map(|answer| (index, answer, source)))
}

fn describe_device(
    device: &Device,
    default_id: Option<&str>,
) -> Result<InputDeviceDescriptor, CpalCaptureError> {
    let stable_id = device
        .id()
        .map_err(|_| CpalCaptureError::DeviceDescriptionUnavailable)?
        .to_string();
    let description = device
        .description()
        .map_err(|_| CpalCaptureError::DeviceDescriptionUnavailable)?;
    let default_config = device
        .default_input_config()
        .ok()
        .and_then(|config| native_config(&config).ok());
    Ok(InputDeviceDescriptor {
        is_default: default_id == Some(stable_id.as_str()),
        stable_id,
        display_name: description.name().to_owned(),
        default_config,
    })
}

fn native_config(
    config: &cpal::SupportedStreamConfig,
) -> Result<NativeStreamConfig, CpalCaptureError> {
    let format = match config.sample_format() {
        SampleFormat::F32 => NativeSampleFormat::F32,
        SampleFormat::I16 => NativeSampleFormat::I16,
        SampleFormat::U16 => NativeSampleFormat::U16,
        _ => return Err(CpalCaptureError::UnsupportedSampleFormat),
    };
    let sample_rate_hz =
        NonZeroU32::new(config.sample_rate()).ok_or(CpalCaptureError::DefaultConfigUnavailable)?;
    let channels =
        NonZeroU16::new(config.channels()).ok_or(CpalCaptureError::DefaultConfigUnavailable)?;
    Ok(NativeStreamConfig::new(format, sample_rate_hz, channels))
}

pub struct CpalCaptureSession {
    identity: CaptureIdentity,
    native: NativeStreamConfig,
    faulted: Arc<AtomicBool>,
    stream: Option<Stream>,
}

impl CpalCaptureSession {
    /// Opens and starts the selected input device using its default native format.
    ///
    /// # Errors
    ///
    /// Returns a sanitized adapter error when selection, format discovery, stream
    /// construction, or stream start fails.
    pub fn start(
        request: &CpalCaptureRequest,
        callback: CaptureCallback,
    ) -> Result<Self, CpalCaptureError> {
        let host = cpal::default_host();
        let device = host
            .input_devices()
            .map_err(|_| CpalCaptureError::EnumerationFailed)?
            .find(|device| {
                device
                    .id()
                    .is_ok_and(|id| id.to_string() == request.device_stable_id)
            })
            .ok_or(CpalCaptureError::DeviceUnavailable)?;
        let supported = device
            .default_input_config()
            .map_err(|_| CpalCaptureError::DefaultConfigUnavailable)?;
        Self::start_on(&device, supported, request.identity, callback)
    }

    /// Starts `input` without walking the devices again. See [`SelectedInput`].
    ///
    /// # Errors
    ///
    /// Returns a sanitized adapter error when stream construction or start
    /// fails.
    pub fn start_selected(
        input: SelectedInput,
        identity: CaptureIdentity,
        callback: CaptureCallback,
    ) -> Result<Self, CpalCaptureError> {
        let SelectedInput {
            device, supported, ..
        } = input;
        Self::start_on(&device, supported, identity, callback)
    }

    fn start_on(
        device: &Device,
        supported: cpal::SupportedStreamConfig,
        identity: CaptureIdentity,
        callback: CaptureCallback,
    ) -> Result<Self, CpalCaptureError> {
        let native = native_config(&supported)?;
        let faulted = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(
                device,
                supported.config(),
                callback,
                Arc::clone(&faulted),
                started,
            ),
            SampleFormat::I16 => build_stream::<i16>(
                device,
                supported.config(),
                callback,
                Arc::clone(&faulted),
                started,
            ),
            SampleFormat::U16 => build_stream::<u16>(
                device,
                supported.config(),
                callback,
                Arc::clone(&faulted),
                started,
            ),
            _ => return Err(CpalCaptureError::UnsupportedSampleFormat),
        }?;
        stream
            .play()
            .map_err(|_| CpalCaptureError::StreamStartFailed)?;
        Ok(Self {
            identity,
            native,
            faulted,
            stream: Some(stream),
        })
    }

    pub const fn identity(&self) -> CaptureIdentity {
        self.identity
    }

    pub const fn native_config(&self) -> NativeStreamConfig {
        self.native
    }

    pub fn poll_fault(&self) -> Option<CaptureFault> {
        self.faulted
            .swap(false, Ordering::AcqRel)
            .then_some(CaptureFault {
                identity: self.identity,
            })
    }

    pub fn stop(&mut self) {
        self.stream.take();
    }
}

impl Drop for CpalCaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn build_stream<T>(
    device: &Device,
    config: StreamConfig,
    mut callback: CaptureCallback,
    faulted: Arc<AtomicBool>,
    started: Instant,
) -> Result<Stream, CpalCaptureError>
where
    T: cpal::SizedSample + CpalSample,
{
    let channels = u64::from(config.channels);
    let mut first_frame_index = 0_u64;
    device
        .build_input_stream::<T, _, _>(
            config,
            move |samples, _| {
                let stamp = CallbackStamp {
                    first_frame_index,
                    capture_monotonic_ns: u64::try_from(started.elapsed().as_nanos())
                        .unwrap_or(u64::MAX),
                    discontinuity: AudioDiscontinuity::NONE,
                };
                T::write(&mut callback, samples, stamp);
                first_frame_index =
                    first_frame_index.saturating_add(samples.len() as u64 / channels);
            },
            move |_| {
                faulted.store(true, Ordering::Release);
            },
            Some(Duration::from_secs(5)),
        )
        .map_err(|_| CpalCaptureError::StreamBuildFailed)
}

trait CpalSample: cpal::SizedSample {
    fn write(callback: &mut CaptureCallback, samples: &[Self], stamp: CallbackStamp);
}

impl CpalSample for f32 {
    fn write(callback: &mut CaptureCallback, samples: &[Self], stamp: CallbackStamp) {
        callback.write_f32(samples, stamp);
    }
}

impl CpalSample for i16 {
    fn write(callback: &mut CaptureCallback, samples: &[Self], stamp: CallbackStamp) {
        callback.write_i16(samples, stamp);
    }
}

impl CpalSample for u16 {
    fn write(callback: &mut CaptureCallback, samples: &[Self], stamp: CallbackStamp) {
        callback.write_u16(samples, stamp);
    }
}
