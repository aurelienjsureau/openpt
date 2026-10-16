use crate::ara_sys as a;
use openpt_dsp::{AraClip, AraTrack, SampleSource};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, Mutex, PoisonError};

#[repr(C)]
struct UnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct EntryPointVtbl {
    base: UnknownVtbl,
    get_factory: unsafe extern "system" fn(*mut c_void) -> *const a::ARAFactory,
    bind_to_document_controller: unsafe extern "system" fn(*mut c_void, a::ARADocumentControllerRef) -> *const a::ARAPlugInExtensionInstance,
}

#[repr(C)]
struct EntryPoint2Vtbl {
    base: UnknownVtbl,
    bind_with_roles: unsafe extern "system" fn(
        *mut c_void,
        a::ARADocumentControllerRef,
        a::ARAPlugInInstanceRoleFlags,
        a::ARAPlugInInstanceRoleFlags,
    ) -> *const a::ARAPlugInExtensionInstance,
}

fn iid(l1: u32, l2: u32, l3: u32, l4: u32) -> [u8; 16] {
    vst3::uid(l1, l2, l3, l4).map(|b| b as u8)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackRequest {
    Start,
    Stop,
    Locate(f64),
    Cycle(f64, f64),
    EnableCycle(bool),
}

static REQUESTS: Mutex<Vec<PlaybackRequest>> = Mutex::new(Vec::new());

pub fn take_playback_requests() -> Vec<PlaybackRequest> {
    std::mem::take(&mut *REQUESTS.lock().unwrap_or_else(PoisonError::into_inner))
}

fn request(r: PlaybackRequest) {
    REQUESTS.lock().unwrap_or_else(PoisonError::into_inner).push(r);
}

struct Source {
    audio: Arc<dyn SampleSource>,
    _id: CString,
    _mod_id: CString,
    _name: CString,
    araref: a::ARAAudioSourceRef,
    modification: a::ARAAudioModificationRef,
}

struct Region {
    clip: u64,
    key: (u64, i64, i64, i64),
    araref: a::ARAPlaybackRegionRef,
    _name: CString,
}

#[derive(Clone, Copy)]
struct Renderer {
    playback: (a::ARAPlaybackRendererRef, *const a::ARAPlaybackRendererInterface),
    editor: (a::ARAEditorRendererRef, *const a::ARAEditorRendererInterface),
    view: (a::ARAEditorViewRef, *const a::ARAEditorViewInterface),
}

struct Sequence {
    araref: a::ARARegionSequenceRef,
    _name: CString,
    regions: Vec<Region>,
    renderers: Vec<Renderer>,
}

impl Sequence {
    fn notify_selection(&self) {
        let regions: Vec<a::ARAPlaybackRegionRef> = self.regions.iter().map(|r| r.araref).collect();
        let sequences = [self.araref];
        let sel = a::ARAViewSelection {
            structSize: std::mem::size_of::<a::ARAViewSelection>(),
            playbackRegionRefsCount: regions.len(),
            playbackRegionRefs: regions.as_ptr(),
            regionSequenceRefsCount: sequences.len(),
            regionSequenceRefs: sequences.as_ptr(),
            timeRange: std::ptr::null(),
        };
        for rd in &self.renderers {
            unsafe {
                if let Some(f) = rd.view.1.as_ref().and_then(|x| x.notifySelection) {
                    f(rd.view.0, &sel);
                }
            }
        }
    }
}

struct Doc {
    ctrl: a::ARADocumentControllerRef,
    iface: *const a::ARADocumentControllerInterface,
    archive_id: a::ARAPersistentID,
    context: a::ARAMusicalContextRef,
    sources: HashMap<(u64, usize), Box<Source>>,
    tracks: HashMap<u64, Sequence>,
    name: String,
    restore: Option<Vec<u8>>,
    _host: Box<a::ARADocumentControllerHostInstance>,
}

unsafe impl Send for Doc {}

static DOCS: Mutex<Vec<(String, Doc)>> = Mutex::new(Vec::new());
static STARTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

static ASSERT: a::ARAAssertFunction = Some(on_assert);
static PENDING: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());

pub fn set_pending_archives(archives: Vec<(String, Vec<u8>)>) {
    *PENDING.lock().unwrap_or_else(PoisonError::into_inner) = archives;
}

fn take_pending(key: &str) -> Option<Vec<u8>> {
    let mut p = PENDING.lock().unwrap_or_else(PoisonError::into_inner);
    let i = p.iter().position(|(k, _)| k == key)?;
    Some(p.remove(i).1)
}

unsafe extern "C" fn on_assert(_category: a::ARAAssertCategory, _arg: *const c_void, diagnosis: *const std::ffi::c_char) {
    let msg = if diagnosis.is_null() { String::new() } else { unsafe { CStr::from_ptr(diagnosis) }.to_string_lossy().into_owned() };
    log::warn!("ARA assert: {msg}");
}

struct Reader {
    source: *const Source,
    f64: bool,
}

unsafe extern "C" fn create_reader(
    _: a::ARAAudioAccessControllerHostRef,
    src: a::ARAAudioSourceHostRef,
    use64: a::ARABool,
) -> a::ARAAudioReaderHostRef {
    Box::into_raw(Box::new(Reader { source: src.cast(), f64: use64 != 0 })).cast()
}

unsafe extern "C" fn read_samples(
    _: a::ARAAudioAccessControllerHostRef,
    reader: a::ARAAudioReaderHostRef,
    pos: a::ARASamplePosition,
    count: a::ARASampleCount,
    buffers: *const *mut c_void,
) -> a::ARABool {
    let Some(r) = (unsafe { reader.cast::<Reader>().as_ref() }) else { return 0 };
    let Some(src) = (unsafe { r.source.as_ref() }) else { return 0 };
    let Ok(count) = usize::try_from(count) else { return 0 };
    for ch in 0..src.audio.num_channels() {
        let buf = unsafe { *buffers.add(ch) };
        if buf.is_null() {
            return 0;
        }
        let data = src.audio.channel(ch).unwrap_or(&[]);
        let at = |i: usize| -> f32 { usize::try_from(pos).ok().and_then(|p| data.get(p + i)).copied().unwrap_or(0.0) };
        if r.f64 {
            let out = unsafe { std::slice::from_raw_parts_mut(buf.cast::<f64>(), count) };
            for (i, o) in out.iter_mut().enumerate() {
                *o = f64::from(at(i));
            }
        } else {
            let out = unsafe { std::slice::from_raw_parts_mut(buf.cast::<f32>(), count) };
            for (i, o) in out.iter_mut().enumerate() {
                *o = at(i);
            }
        }
    }
    1
}

unsafe extern "C" fn destroy_reader(_: a::ARAAudioAccessControllerHostRef, reader: a::ARAAudioReaderHostRef) {
    if !reader.is_null() {
        drop(unsafe { Box::from_raw(reader.cast::<Reader>()) });
    }
}

static AUDIO_ACCESS: a::ARAAudioAccessControllerInterface = a::ARAAudioAccessControllerInterface {
    structSize: std::mem::size_of::<a::ARAAudioAccessControllerInterface>(),
    createAudioReaderForSource: Some(create_reader),
    readAudioSamples: Some(read_samples),
    destroyAudioReader: Some(destroy_reader),
};

struct Archive {
    bytes: Vec<u8>,
    id: a::ARAPersistentID,
}

unsafe extern "C" fn archive_size(_: a::ARAArchivingControllerHostRef, r: a::ARAArchiveReaderHostRef) -> a::ARASize {
    unsafe { r.cast::<Archive>().as_ref() }.map_or(0, |a| a.bytes.len())
}

unsafe extern "C" fn archive_read(
    _: a::ARAArchivingControllerHostRef,
    r: a::ARAArchiveReaderHostRef,
    pos: a::ARASize,
    len: a::ARASize,
    out: *mut a::ARAByte,
) -> a::ARABool {
    let Some(ar) = (unsafe { r.cast::<Archive>().as_ref() }) else { return 0 };
    let Some(src) = ar.bytes.get(pos..pos.saturating_add(len)) else { return 0 };
    unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), out, len) };
    1
}

unsafe extern "C" fn archive_write(
    _: a::ARAArchivingControllerHostRef,
    w: a::ARAArchiveWriterHostRef,
    pos: a::ARASize,
    len: a::ARASize,
    data: *const a::ARAByte,
) -> a::ARABool {
    let Some(ar) = (unsafe { w.cast::<Archive>().as_mut() }) else { return 0 };
    let end = pos.saturating_add(len);
    if end > 1 << 31 {
        return 0;
    }
    if ar.bytes.len() < end {
        ar.bytes.resize(end, 0);
    }
    unsafe { std::ptr::copy_nonoverlapping(data, ar.bytes.as_mut_ptr().add(pos), len) };
    1
}

unsafe extern "C" fn archive_progress(_: a::ARAArchivingControllerHostRef, _: f32) {}

unsafe extern "C" fn archive_id(_: a::ARAArchivingControllerHostRef, r: a::ARAArchiveReaderHostRef) -> a::ARAPersistentID {
    unsafe { r.cast::<Archive>().as_ref() }.map_or(std::ptr::null(), |a| a.id)
}

static ARCHIVING: a::ARAArchivingControllerInterface = a::ARAArchivingControllerInterface {
    structSize: std::mem::size_of::<a::ARAArchivingControllerInterface>(),
    getArchiveSize: Some(archive_size),
    readBytesFromArchive: Some(archive_read),
    writeBytesToArchive: Some(archive_write),
    notifyDocumentArchivingProgress: Some(archive_progress),
    notifyDocumentUnarchivingProgress: Some(archive_progress),
    getDocumentArchiveID: Some(archive_id),
};

unsafe extern "C" fn no_ctx_content(_: a::ARAContentAccessControllerHostRef, _: a::ARAMusicalContextHostRef, _: a::ARAContentType) -> a::ARABool {
    0
}
unsafe extern "C" fn no_ctx_grade(
    _: a::ARAContentAccessControllerHostRef,
    _: a::ARAMusicalContextHostRef,
    _: a::ARAContentType,
) -> a::ARAContentGrade {
    0
}
unsafe extern "C" fn no_ctx_reader(
    _: a::ARAContentAccessControllerHostRef,
    _: a::ARAMusicalContextHostRef,
    _: a::ARAContentType,
    _: *const a::ARAContentTimeRange,
) -> a::ARAContentReaderHostRef {
    std::ptr::null_mut()
}
unsafe extern "C" fn no_src_content(_: a::ARAContentAccessControllerHostRef, _: a::ARAAudioSourceHostRef, _: a::ARAContentType) -> a::ARABool {
    0
}
unsafe extern "C" fn no_src_grade(_: a::ARAContentAccessControllerHostRef, _: a::ARAAudioSourceHostRef, _: a::ARAContentType) -> a::ARAContentGrade {
    0
}
unsafe extern "C" fn no_src_reader(
    _: a::ARAContentAccessControllerHostRef,
    _: a::ARAAudioSourceHostRef,
    _: a::ARAContentType,
    _: *const a::ARAContentTimeRange,
) -> a::ARAContentReaderHostRef {
    std::ptr::null_mut()
}
unsafe extern "C" fn no_count(_: a::ARAContentAccessControllerHostRef, _: a::ARAContentReaderHostRef) -> a::ARAInt32 {
    0
}
unsafe extern "C" fn no_event(_: a::ARAContentAccessControllerHostRef, _: a::ARAContentReaderHostRef, _: a::ARAInt32) -> *const c_void {
    std::ptr::null()
}
unsafe extern "C" fn no_destroy(_: a::ARAContentAccessControllerHostRef, _: a::ARAContentReaderHostRef) {}

static CONTENT: a::ARAContentAccessControllerInterface = a::ARAContentAccessControllerInterface {
    structSize: std::mem::size_of::<a::ARAContentAccessControllerInterface>(),
    isMusicalContextContentAvailable: Some(no_ctx_content),
    getMusicalContextContentGrade: Some(no_ctx_grade),
    createMusicalContextContentReader: Some(no_ctx_reader),
    isAudioSourceContentAvailable: Some(no_src_content),
    getAudioSourceContentGrade: Some(no_src_grade),
    createAudioSourceContentReader: Some(no_src_reader),
    getContentReaderEventCount: Some(no_count),
    getContentReaderDataForEvent: Some(no_event),
    destroyContentReader: Some(no_destroy),
};

unsafe extern "C" fn on_analysis(_: a::ARAModelUpdateControllerHostRef, _: a::ARAAudioSourceHostRef, _: a::ARAAnalysisProgressState, _: f32) {}
unsafe extern "C" fn on_source_changed(
    _: a::ARAModelUpdateControllerHostRef,
    _: a::ARAAudioSourceHostRef,
    _: *const a::ARAContentTimeRange,
    _: a::ARAContentUpdateFlags,
) {
}
unsafe extern "C" fn on_mod_changed(
    _: a::ARAModelUpdateControllerHostRef,
    _: a::ARAAudioModificationHostRef,
    _: *const a::ARAContentTimeRange,
    _: a::ARAContentUpdateFlags,
) {
}
unsafe extern "C" fn on_region_changed(
    _: a::ARAModelUpdateControllerHostRef,
    _: a::ARAPlaybackRegionHostRef,
    _: *const a::ARAContentTimeRange,
    _: a::ARAContentUpdateFlags,
) {
}
unsafe extern "C" fn on_doc_changed(_: a::ARAModelUpdateControllerHostRef) {}

static MODEL_UPDATE: a::ARAModelUpdateControllerInterface = a::ARAModelUpdateControllerInterface {
    structSize: std::mem::size_of::<a::ARAModelUpdateControllerInterface>(),
    notifyAudioSourceAnalysisProgress: Some(on_analysis),
    notifyAudioSourceContentChanged: Some(on_source_changed),
    notifyAudioModificationContentChanged: Some(on_mod_changed),
    notifyPlaybackRegionContentChanged: Some(on_region_changed),
    notifyDocumentDataChanged: Some(on_doc_changed),
};

unsafe extern "C" fn req_start(_: a::ARAPlaybackControllerHostRef) {
    request(PlaybackRequest::Start);
}
unsafe extern "C" fn req_stop(_: a::ARAPlaybackControllerHostRef) {
    request(PlaybackRequest::Stop);
}
unsafe extern "C" fn req_locate(_: a::ARAPlaybackControllerHostRef, t: a::ARATimePosition) {
    request(PlaybackRequest::Locate(t));
}
unsafe extern "C" fn req_cycle(_: a::ARAPlaybackControllerHostRef, start: a::ARATimePosition, dur: a::ARATimeDuration) {
    request(PlaybackRequest::Cycle(start, dur));
}
unsafe extern "C" fn req_enable_cycle(_: a::ARAPlaybackControllerHostRef, on: a::ARABool) {
    request(PlaybackRequest::EnableCycle(on != 0));
}

static PLAYBACK: a::ARAPlaybackControllerInterface = a::ARAPlaybackControllerInterface {
    structSize: std::mem::size_of::<a::ARAPlaybackControllerInterface>(),
    requestStartPlayback: Some(req_start),
    requestStopPlayback: Some(req_stop),
    requestSetPlaybackPosition: Some(req_locate),
    requestSetCycleRange: Some(req_cycle),
    requestEnableCycle: Some(req_enable_cycle),
};

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn factory_key(f: &a::ARAFactory) -> String {
    let id = f.factoryID;
    if id.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned()
}

fn document<'d>(docs: &'d mut Vec<(String, Doc)>, factory: &a::ARAFactory) -> Option<&'d mut Doc> {
    let key = factory_key(factory);
    if !docs.iter().any(|(k, _)| *k == key) {
        let mut started = STARTED.lock().unwrap_or_else(PoisonError::into_inner);
        if !started.contains(&key) {
            let lowest = factory.lowestSupportedApiGeneration;
            let highest = factory.highestSupportedApiGeneration;
            let generation = highest.min(a::kARAAPIGeneration_2_3_Final).max(lowest);
            if generation < a::kARAAPIGeneration_2_0_Final {
                log::warn!("ARA: {key} needs an ARA generation this host does not offer");
                return None;
            }
            let cfg = a::ARAInterfaceConfiguration {
                structSize: std::mem::size_of::<a::ARAInterfaceConfiguration>(),
                desiredApiGeneration: generation,
                assertFunctionAddress: std::ptr::addr_of!(ASSERT).cast_mut(),
            };
            let init = factory.initializeARAWithConfiguration;
            unsafe { init?(&cfg) };
            started.push(key.clone());
        }
        let mut host = Box::new(a::ARADocumentControllerHostInstance {
            structSize: std::mem::size_of::<a::ARADocumentControllerHostInstance>(),
            audioAccessControllerHostRef: std::ptr::null_mut(),
            audioAccessControllerInterface: &AUDIO_ACCESS,
            archivingControllerHostRef: std::ptr::null_mut(),
            archivingControllerInterface: &ARCHIVING,
            contentAccessControllerHostRef: std::ptr::null_mut(),
            contentAccessControllerInterface: &CONTENT,
            modelUpdateControllerHostRef: std::ptr::null_mut(),
            modelUpdateControllerInterface: &MODEL_UPDATE,
            playbackControllerHostRef: std::ptr::null_mut(),
            playbackControllerInterface: &PLAYBACK,
        });
        let name = cstr("OpenPT");
        let props = a::ARADocumentProperties { structSize: std::mem::size_of::<a::ARADocumentProperties>(), name: name.as_ptr() };
        let create = factory.createDocumentControllerWithDocument;
        let inst = unsafe { create?(&*host, &props) };
        let inst = unsafe { inst.as_ref() }?;
        let (ctrl, iface) = (inst.documentControllerRef, inst.documentControllerInterface);
        let i = unsafe { iface.as_ref() }?;
        let ctx_name = cstr("Session");
        let ctx_props = a::ARAMusicalContextProperties {
            structSize: std::mem::size_of::<a::ARAMusicalContextProperties>(),
            name: ctx_name.as_ptr(),
            orderIndex: 0,
            color: std::ptr::null(),
        };
        let context = unsafe {
            (i.beginEditing?)(ctrl);
            let c = (i.createMusicalContext?)(ctrl, std::ptr::null_mut(), &ctx_props);
            (i.endEditing?)(ctrl);
            c
        };
        let _ = &mut host;
        let restore = take_pending(&key);
        let pname = if factory.plugInName.is_null() {
            key.clone()
        } else {
            unsafe { CStr::from_ptr(factory.plugInName) }.to_string_lossy().into_owned()
        };
        docs.push((
            key.clone(),
            Doc {
                name: pname,
                ctrl,
                iface,
                archive_id: factory.documentArchiveID,
                context,
                sources: HashMap::new(),
                tracks: HashMap::new(),
                restore,
                _host: host,
            },
        ));
    }
    docs.iter_mut().find(|(k, _)| *k == key).map(|(_, d)| d)
}

impl Doc {
    fn iface(&self) -> Option<&a::ARADocumentControllerInterface> {
        unsafe { self.iface.as_ref() }
    }

    fn ensure_source(&mut self, c: &AraClip) -> Option<a::ARAAudioModificationRef> {
        let skey = (c.source, c.audio.frames());
        if let Some(s) = self.sources.get(&skey) {
            return Some(s.modification);
        }
        let (ctrl, i) = (self.ctrl, *self.iface()?);
        let id = cstr(&format!("openpt.source.{}", c.source));
        let name = cstr(&c.source_name);
        let mod_id = cstr(&format!("openpt.modification.{}", c.source));
        let mut src = Box::new(Source {
            audio: Arc::clone(&c.audio),
            _id: id,
            _mod_id: mod_id,
            _name: name,
            araref: std::ptr::null_mut(),
            modification: std::ptr::null_mut(),
        });
        let props = a::ARAAudioSourceProperties {
            structSize: std::mem::size_of::<a::ARAAudioSourceProperties>(),
            name: src._name.as_ptr(),
            persistentID: src._id.as_ptr(),
            sampleCount: i64::try_from(c.audio.frames()).unwrap_or(0),
            sampleRate: f64::from(c.audio.sample_rate()),
            channelCount: i32::try_from(c.audio.num_channels()).unwrap_or(1),
            merits64BitSamples: 0,
            channelArrangementDataType: a::kARAChannelArrangementUndefined,
            channelArrangement: std::ptr::null(),
        };
        let mod_props = a::ARAAudioModificationProperties {
            structSize: std::mem::size_of::<a::ARAAudioModificationProperties>(),
            name: src._name.as_ptr(),
            persistentID: src._mod_id.as_ptr(),
        };
        let host_ref: a::ARAAudioSourceHostRef = std::ptr::addr_of!(*src).cast_mut().cast();
        unsafe {
            src.araref = (i.createAudioSource?)(ctrl, host_ref, &props);
            (i.enableAudioSourceSamplesAccess?)(ctrl, src.araref, 1);
            src.modification = (i.createAudioModification?)(ctrl, src.araref, std::ptr::null_mut(), &mod_props);
        }
        let m = src.modification;
        self.sources.insert(skey, src);
        Some(m)
    }

    fn sync(&mut self, t: &AraTrack) {
        let Some(i) = self.iface().copied() else { return };
        let ctrl = self.ctrl;
        unsafe {
            if let Some(f) = i.beginEditing {
                f(ctrl);
            }
        }
        if !self.tracks.contains_key(&t.track) {
            let name = cstr(&t.name);
            let props = a::ARARegionSequenceProperties {
                structSize: std::mem::size_of::<a::ARARegionSequenceProperties>(),
                name: name.as_ptr(),
                orderIndex: i32::try_from(t.track % 100_000).unwrap_or(0),
                musicalContextRef: self.context,
                color: std::ptr::null(),
            };
            let araref = unsafe { i.createRegionSequence.map_or(std::ptr::null_mut(), |f| f(ctrl, std::ptr::null_mut(), &props)) };
            self.tracks.insert(t.track, Sequence { araref, _name: name, regions: Vec::new(), renderers: Vec::new() });
        }
        let mods: Vec<Option<a::ARAAudioModificationRef>> = t.clips.iter().map(|c| self.ensure_source(c)).collect();
        let context = self.context;
        let Some(seq) = self.tracks.get_mut(&t.track) else { return };
        let rate = t.sample_rate.max(1.0);
        let key = |c: &AraClip| (c.source, c.offset, c.length, c.start);
        let (keep, gone): (Vec<Region>, Vec<Region>) =
            std::mem::take(&mut seq.regions).into_iter().partition(|r| t.clips.iter().any(|c| c.clip == r.clip && key(c) == r.key));
        for r in gone {
            for rd in &seq.renderers {
                unsafe {
                    if let Some(pi) = rd.playback.1.as_ref().and_then(|x| x.removePlaybackRegion) {
                        pi(rd.playback.0, r.araref);
                    }
                    if let Some(ei) = rd.editor.1.as_ref().and_then(|x| x.removePlaybackRegion) {
                        ei(rd.editor.0, r.araref);
                    }
                }
            }
            unsafe {
                if let Some(f) = i.destroyPlaybackRegion {
                    f(ctrl, r.araref);
                }
            }
        }
        seq.regions = keep;
        for (c, m) in t.clips.iter().zip(mods) {
            let Some(m) = m else { continue };
            if seq.regions.iter().any(|r| r.clip == c.clip) {
                continue;
            }
            let src_rate = f64::from(c.audio.sample_rate()).max(1.0);
            let name = cstr(&c.name);
            let props = a::ARAPlaybackRegionProperties {
                structSize: std::mem::size_of::<a::ARAPlaybackRegionProperties>(),
                transformationFlags: 0,
                startInModificationTime: c.offset as f64 / src_rate,
                durationInModificationTime: c.length as f64 / rate,
                startInPlaybackTime: c.start as f64 / rate,
                durationInPlaybackTime: c.length as f64 / rate,
                musicalContextRef: context,
                regionSequenceRef: seq.araref,
                name: name.as_ptr(),
                color: std::ptr::null(),
            };
            let araref = unsafe { i.createPlaybackRegion.map_or(std::ptr::null_mut(), |f| f(ctrl, m, std::ptr::null_mut(), &props)) };
            if araref.is_null() {
                continue;
            }
            for rd in &seq.renderers {
                unsafe {
                    if let Some(pi) = rd.playback.1.as_ref().and_then(|x| x.addPlaybackRegion) {
                        pi(rd.playback.0, araref);
                    }
                    if let Some(ei) = rd.editor.1.as_ref().and_then(|x| x.addPlaybackRegion) {
                        ei(rd.editor.0, araref);
                    }
                }
            }
            seq.regions.push(Region { clip: c.clip, key: key(c), araref, _name: name });
        }
        unsafe {
            if let Some(f) = i.endEditing {
                f(ctrl);
            }
        }
        if let Some(seq) = self.tracks.get(&t.track) {
            seq.notify_selection();
        }
    }
}

pub(crate) unsafe fn bind(component: *mut c_void, track: &AraTrack) -> Option<Binding> {
    if component.is_null() {
        return None;
    }
    let query = |iid: [u8; 16]| -> Option<*mut c_void> {
        let mut out: *mut c_void = std::ptr::null_mut();
        let vt = unsafe { &**(component as *const *const UnknownVtbl) };
        (unsafe { (vt.query_interface)(component, &iid, &mut out) } == 0 && !out.is_null()).then_some(out)
    };
    let Some(entry) = query(iid(0x1281_4E54, 0xA1CE_4076, 0x82B9_6813, 0x1695_0BD6)) else {
        log::info!("ARA: no IPlugInEntryPoint on this component");
        return None;
    };
    let evt = unsafe { &**(entry as *const *const EntryPointVtbl) };
    let factory = unsafe { (evt.get_factory)(entry).as_ref() };
    let Some(factory) = factory else {
        unsafe { (evt.base.release)(entry) };
        return None;
    };
    let mut docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
    log::info!("ARA: factory {} ({} clips on track {})", factory_key(factory), track.clips.len(), track.track);
    let Some(doc) = document(&mut docs, factory) else {
        log::warn!("ARA: no document for {}", factory_key(factory));
        unsafe { (evt.base.release)(entry) };
        return None;
    };
    doc.sync(track);
    let roles = a::kARAPlaybackRendererRole | a::kARAEditorRendererRole | a::kARAEditorViewRole;
    let ext = match query(iid(0xCD9A_5913, 0xC9EB_46D7, 0x96CA_53AD, 0xD1DB_89F5)) {
        Some(e2) => {
            let vt2 = unsafe { &**(e2 as *const *const EntryPoint2Vtbl) };
            let x = unsafe { (vt2.bind_with_roles)(e2, doc.ctrl, roles, roles) };
            unsafe { (vt2.base.release)(e2) };
            x
        }
        None => unsafe { (evt.bind_to_document_controller)(entry, doc.ctrl) },
    };
    unsafe { (evt.base.release)(entry) };
    let Some(ext) = (unsafe { ext.as_ref() }) else {
        log::warn!("ARA: the plug-in refused the binding");
        return None;
    };
    log::info!("ARA: bound to track {}", track.track);
    let rd =
        Renderer {
        playback: (ext.playbackRendererRef, ext.playbackRendererInterface),
        editor: (ext.editorRendererRef, ext.editorRendererInterface),
        view: (ext.editorViewRef, ext.editorViewInterface),
    };
    if let Some(seq) = doc.tracks.get_mut(&track.track) {
        for r in &seq.regions {
            unsafe {
                if let Some(pi) = rd.playback.1.as_ref().and_then(|x| x.addPlaybackRegion) {
                    pi(rd.playback.0, r.araref);
                }
                if let Some(ei) = rd.editor.1.as_ref().and_then(|x| x.addPlaybackRegion) {
                    ei(rd.editor.0, r.araref);
                }
            }
        }
        seq.renderers.push(rd);
        seq.notify_selection();
    }
    Some(Binding { factory: factory_key(factory), track: track.track, renderer: rd })
}

pub struct Binding {
    factory: String,
    track: u64,
    renderer: Renderer,
}

unsafe impl Send for Binding {}

impl Binding {
    pub fn update(&self, track: &AraTrack) {
        let mut docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, d)) = docs.iter_mut().find(|(k, _)| *k == self.factory) {
            d.sync(track);
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        let mut docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(seq) = docs.iter_mut().find(|(k, _)| *k == self.factory).and_then(|(_, d)| d.tracks.get_mut(&self.track)) {
            seq.renderers.retain(|r| r.playback.0 != self.renderer.playback.0);
        }
    }
}

pub fn store_documents() -> Vec<(String, Vec<u8>)> {
    let docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut out = Vec::new();
    for (k, d) in docs.iter() {
        let Some(i) = d.iface() else { continue };
        let mut ar = Box::new(Archive { bytes: Vec::new(), id: d.archive_id });
        let w: a::ARAArchiveWriterHostRef = std::ptr::addr_of_mut!(*ar).cast();
        let stored = unsafe {
            match (i.storeObjectsToArchive, i.storeDocumentToArchive) {
                (Some(f), _) => f(d.ctrl, w, std::ptr::null()) != 0,
                (None, Some(f)) => f(d.ctrl, w) != 0,
                _ => false,
            }
        };
        if stored {
            out.push((k.clone(), std::mem::take(&mut ar.bytes)));
        }
    }
    out
}

pub fn bound_tracks() -> Vec<u64> {
    let docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut t: Vec<u64> = docs.iter().flat_map(|(_, d)| d.tracks.iter().filter(|(_, s)| !s.renderers.is_empty()).map(|(k, _)| *k)).collect();
    t.sort_unstable();
    t.dedup();
    t
}

pub fn sync_track(track: &AraTrack) {
    let mut docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
    for (_, d) in docs.iter_mut() {
        if d.tracks.contains_key(&track.track) {
            d.sync(track);
        }
    }
}

pub fn finish_restores() {
    let mut docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
    for (k, d) in docs.iter_mut() {
        if d.tracks.values().all(|s| s.renderers.is_empty()) {
            continue;
        }
        let Some(bytes) = d.restore.take().or_else(|| take_pending(k)) else { continue };
        let Some(i) = d.iface().copied() else { continue };
        let Some(restore) = i.restoreObjectsFromArchive else { continue };
        let mut ar = Box::new(Archive { bytes, id: d.archive_id });
        let r: a::ARAArchiveReaderHostRef = std::ptr::addr_of_mut!(*ar).cast();
        unsafe {
            if let Some(f) = i.beginEditing {
                f(d.ctrl);
            }
            if restore(d.ctrl, r, std::ptr::null()) == 0 {
                log::warn!("ARA: a plug-in could not restore its saved state");
            }
            if let Some(f) = i.endEditing {
                f(d.ctrl);
            }
        }
    }
}

pub fn bindings() -> Vec<(u64, String)> {
    let docs = DOCS.lock().unwrap_or_else(PoisonError::into_inner);
    docs.iter().flat_map(|(_, d)| d.tracks.iter().filter(|(_, s)| !s.renderers.is_empty()).map(|(t, _)| (*t, d.name.clone()))).collect()
}
