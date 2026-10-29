use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::{Mutex, PoisonError};

pub const STANDARD_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
pub const STANDARD_BUFFERS: [u32; 9] = [32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];

pub(crate) const FORMATS: [cpal::SampleFormat; 5] =
    [cpal::SampleFormat::F32, cpal::SampleFormat::I32, cpal::SampleFormat::I24, cpal::SampleFormat::F64, cpal::SampleFormat::I16];

pub(crate) fn format_rank(f: cpal::SampleFormat) -> Option<usize> {
    FORMATS.iter().position(|x| *x == f)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub system: Option<String>,
    pub output: Option<String>,
    pub input: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub max_channels: u16,
    pub sample_rates: Vec<u32>,
    pub buffer_min: Option<u32>,
    pub buffer_max: Option<u32>,
}

impl DeviceInfo {
    pub fn buffer_sizes(&self) -> Vec<u32> {
        STANDARD_BUFFERS.iter().copied().filter(|b| self.buffer_min.is_none_or(|m| *b >= m) && self.buffer_max.is_none_or(|m| *b <= m)).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AudioSystem {
    pub name: String,
    pub is_default: bool,
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
}

#[allow(deprecated)]
fn device_name(d: &cpal::Device) -> Option<String> {
    d.name().ok()
}

fn describe(d: &cpal::Device, default_name: Option<&str>, output: bool) -> Option<DeviceInfo> {
    let name = device_name(d)?;
    let ranges: Vec<cpal::SupportedStreamConfigRange> = if output {
        d.supported_output_configs().ok()?.filter(|c| format_rank(c.sample_format()).is_some()).collect()
    } else {
        d.supported_input_configs().ok()?.filter(|c| format_rank(c.sample_format()).is_some()).collect()
    };
    let max_channels = ranges.iter().map(cpal::SupportedStreamConfigRange::channels).max().unwrap_or(0);
    let sample_rates =
        STANDARD_RATES.iter().copied().filter(|r| ranges.iter().any(|c| c.min_sample_rate().0 <= *r && c.max_sample_rate().0 >= *r)).collect();
    let (mut buffer_min, mut buffer_max) = (None::<u32>, None::<u32>);
    for c in &ranges {
        if let cpal::SupportedBufferSize::Range { min, max } = c.buffer_size() {
            buffer_min = Some(buffer_min.map_or(*min, |m| m.min(*min)));
            buffer_max = Some(buffer_max.map_or(*max, |m| m.max(*max)));
        }
    }
    Some(DeviceInfo { is_default: default_name == Some(name.as_str()), name, max_channels, sample_rates, buffer_min, buffer_max })
}

pub fn audio_systems() -> Vec<AudioSystem> {
    let default_id = cpal::default_host().id();
    cpal::available_hosts()
        .into_iter()
        .filter_map(|id| {
            let host = cached_host(id)?;
            let def_out = host.default_output_device().as_ref().and_then(device_name);
            let def_in = host.default_input_device().as_ref().and_then(device_name);
            let mut outputs: Vec<DeviceInfo> =
                host.output_devices().map(|ds| ds.filter_map(|d| describe(&d, def_out.as_deref(), true)).collect()).unwrap_or_default();
            let mut inputs: Vec<DeviceInfo> =
                host.input_devices().map(|ds| ds.filter_map(|d| describe(&d, def_in.as_deref(), false)).collect()).unwrap_or_default();
            if is_asio(id) {
                remember_asio(&mut outputs, &mut inputs);
            }
            Some(AudioSystem { name: id.name().to_string(), is_default: id == default_id, outputs, inputs })
        })
        .collect()
}

fn is_asio(id: cpal::HostId) -> bool {
    id.name().eq_ignore_ascii_case("ASIO")
}

static ASIO_OPEN: Mutex<Option<cpal::Device>> = Mutex::new(None);

fn cached_host(id: cpal::HostId) -> Option<&'static cpal::Host> {
    static HOSTS: Mutex<Vec<(cpal::HostId, &'static cpal::Host)>> = Mutex::new(Vec::new());
    let mut hosts = HOSTS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, h)) = hosts.iter().find(|(i, _)| *i == id) {
        return Some(*h);
    }
    let host: &'static cpal::Host = Box::leak(Box::new(cpal::host_from_id(id).ok()?));
    hosts.push((id, host));
    Some(host)
}

fn remember_asio(outputs: &mut Vec<DeviceInfo>, inputs: &mut Vec<DeviceInfo>) {
    static SEEN: Mutex<(Vec<DeviceInfo>, Vec<DeviceInfo>)> = Mutex::new((Vec::new(), Vec::new()));
    let merge = |live: &mut Vec<DeviceInfo>, seen: &mut Vec<DeviceInfo>| {
        for d in live.iter() {
            let d = DeviceInfo { is_default: false, ..d.clone() };
            match seen.iter_mut().find(|s| s.name == d.name) {
                Some(s) => *s = d,
                None => seen.push(d),
            }
        }
        for s in seen.iter() {
            if !live.iter().any(|d| d.name == s.name) {
                live.push(s.clone());
            }
        }
    };
    let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    let (o, i) = &mut *seen;
    merge(outputs, o);
    merge(inputs, i);
}

pub(crate) fn host(cfg: &AudioConfig) -> &'static cpal::Host {
    let chosen = cfg
        .system
        .as_deref()
        .and_then(|want| cpal::available_hosts().into_iter().find(|id| id.name().eq_ignore_ascii_case(want)))
        .and_then(cached_host);
    chosen.unwrap_or_else(|| {
        if let Some(s) = &cfg.system {
            log::warn!("audio system `{s}` is not available; using the default");
        }
        cached_host(cpal::default_host().id()).unwrap_or_else(|| Box::leak(Box::new(cpal::default_host())))
    })
}

pub fn release_asio() {
    let old = ASIO_OPEN.lock().unwrap_or_else(PoisonError::into_inner).take();
    drop(old);
}

pub(crate) fn device(host: &cpal::Host, name: Option<&str>, output: bool) -> Option<cpal::Device> {
    if !is_asio(host.id()) {
        return find(host, name, output);
    }
    let mut open = ASIO_OPEN.lock().unwrap_or_else(PoisonError::into_inner);
    if !output && let Some(d) = open.as_ref() {
        let have = device_name(d);
        if let Some(want) = name
            && have.as_deref() != Some(want)
        {
            log::warn!("ASIO input `{want}` cannot run beside the output driver; using {}", have.as_deref().unwrap_or("it"));
        }
        return Some(d.clone());
    }
    let old = open.take();
    drop(old);
    let found = find(host, name, output);
    if let Some(d) = &found {
        let mut outputs: Vec<DeviceInfo> = describe(d, None, true).into_iter().collect();
        let mut inputs: Vec<DeviceInfo> = describe(d, None, false).into_iter().collect();
        remember_asio(&mut outputs, &mut inputs);
    }
    open.clone_from(&found);
    found
}

fn find(host: &cpal::Host, name: Option<&str>, output: bool) -> Option<cpal::Device> {
    if let Some(want) = name {
        let found = if output { host.output_devices().ok() } else { host.input_devices().ok() }
            .and_then(|mut ds| ds.find(|d| device_name(d).as_deref() == Some(want)));
        if found.is_some() {
            return found;
        }
        log::warn!("audio device `{want}` is not available; using the default");
    }
    if output { host.default_output_device() } else { host.default_input_device() }
}

pub(crate) fn buffer_size(want: Option<u32>, supported: &cpal::SupportedBufferSize) -> cpal::BufferSize {
    match (want, supported) {
        (Some(n), cpal::SupportedBufferSize::Range { min, max }) if (*min..=*max).contains(&n) => cpal::BufferSize::Fixed(n),
        (Some(n), cpal::SupportedBufferSize::Unknown) if (16..=8192).contains(&n) => cpal::BufferSize::Fixed(n),
        _ => cpal::BufferSize::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_and_tolerates_missing_fields() {
        let c = AudioConfig {
            system: Some("WASAPI".into()),
            output: Some("Speakers".into()),
            input: None,
            sample_rate: Some(48_000),
            buffer_size: Some(256),
        };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<AudioConfig>(&s).unwrap(), c);
        assert_eq!(serde_json::from_str::<AudioConfig>("{}").unwrap(), AudioConfig::default());
        assert_eq!(serde_json::from_str::<AudioConfig>(r#"{"output":"X","unknown":1}"#).unwrap().output.as_deref(), Some("X"));
    }

    #[test]
    fn buffer_sizes_follow_the_device_range() {
        let r = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(buffer_size(Some(256), &r), cpal::BufferSize::Fixed(256));
        assert_eq!(buffer_size(Some(32), &r), cpal::BufferSize::Default);
        assert_eq!(buffer_size(Some(4096), &r), cpal::BufferSize::Default);
        assert_eq!(buffer_size(None, &r), cpal::BufferSize::Default);
        assert_eq!(buffer_size(Some(128), &cpal::SupportedBufferSize::Unknown), cpal::BufferSize::Fixed(128));
        assert_eq!(buffer_size(Some(1 << 20), &cpal::SupportedBufferSize::Unknown), cpal::BufferSize::Default);
        let d = DeviceInfo {
            name: "x".into(),
            is_default: true,
            max_channels: 2,
            sample_rates: vec![48_000],
            buffer_min: Some(64),
            buffer_max: Some(1024),
        };
        assert_eq!(d.buffer_sizes(), [64, 128, 256, 512, 1024]);
        let open = DeviceInfo { buffer_min: None, buffer_max: None, ..d };
        assert_eq!(open.buffer_sizes().len(), STANDARD_BUFFERS.len());
    }

    #[test]
    fn unknown_systems_and_devices_fall_back_to_the_defaults() {
        let cfg = AudioConfig { system: Some("No Such System".into()), ..Default::default() };
        assert_eq!(host(&cfg).id(), cpal::default_host().id());
        let h = cpal::default_host();
        let fallback = device(&h, Some("No Such Device 1234"), true).as_ref().and_then(device_name);
        assert_eq!(fallback, h.default_output_device().as_ref().and_then(device_name));
    }

    #[test]
    fn enumeration_does_not_fail() {
        let systems = audio_systems();
        assert!(systems.iter().all(|s| !s.name.is_empty()));
        assert!(systems.iter().filter(|s| s.is_default).count() <= 1);
    }
}
