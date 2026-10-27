#![allow(clippy::unnecessary_cast)]

use crate::{Vst3Error, scan};
use openpt_dsp::Transport;
use std::ffi::{c_char, c_int, c_void};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, TryLockError, mpsc};
use std::time::Duration;
use vst3::Steinberg::Vst::{self as sv};
use vst3::Steinberg::Vst::{
    IAudioProcessorTrait, IComponentHandlerTrait, IComponentTrait, IConnectionPointTrait, IEditControllerTrait, IEventListTrait,
    IHostApplicationTrait, IMidiMappingTrait, IParamValueQueueTrait, IParameterChangesTrait,
};
use vst3::Steinberg::{self as sb};
use vst3::Steinberg::{
    FUnknown, IBStreamTrait, IPlugFrameTrait, IPlugViewContentScaleSupportTrait, IPlugViewTrait, IPluginBaseTrait, IPluginFactory2Trait,
    IPluginFactoryTrait,
};
use vst3::{Class, ComPtr, ComWrapper, Interface};

const MAX_CLASSES: i32 = 1024;
pub(crate) const MAX_PARAMS: i32 = 4096;
const MAX_BUSES: i32 = 16;
const MAX_BUS_CHANNELS: u32 = 64;
pub(crate) const EVENT_CAP: usize = 1024;
pub(crate) const PARAM_QUEUE_CAP: usize = 1024;
const OUTPUTS_CAP: usize = 256;
const MAX_STATE_BYTES: usize = 64 << 20;
const EDIT_CAP: usize = 256;
const STATE_MAGIC: &[u8; 4] = b"SCV3";

fn read_c8(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn read_c16(buf: &[u16]) -> String {
    let units: Vec<u16> = buf.iter().take_while(|c| **c != 0).copied().collect();
    String::from_utf16_lossy(&units).trim().to_string()
}

fn write_c16(s: &str, dst: &mut [u16]) {
    let Some(room) = dst.len().checked_sub(1) else { return };
    let mut n = 0;
    for (d, u) in dst.iter_mut().zip(s.encode_utf16().take(room)) {
        *d = u;
        n += 1;
    }
    if let Some(z) = dst.get_mut(n) {
        *z = 0;
    }
}

fn ok(r: sb::tresult) -> bool {
    r == sb::kResultOk
}

pub(crate) const AFTERTOUCH: usize = sv::ControllerNumbers_::kAfterTouch as usize;
pub(crate) const PITCH_BEND: usize = sv::ControllerNumbers_::kPitchBend as usize;

#[derive(Debug, Clone)]
pub(crate) struct RawClass {
    pub cid: [u8; 16],
    pub category: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sdk_version: String,
    pub sub_categories: String,
}

pub(crate) struct Bundle {
    factory: ComPtr<sb::IPluginFactory>,
    exit: Option<ModuleExit>,
    entered_on: EntryThread,
    leaked: AtomicBool,
    _lib: ManuallyDrop<libloading::Library>,
}

fn loaded() -> &'static Mutex<Vec<(PathBuf, Arc<Bundle>)>> {
    static L: OnceLock<Mutex<Vec<(PathBuf, Arc<Bundle>)>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Vec::new()))
}

static MODULES_EXITED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryThread {
    Main,
    Modules,
}

fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

static MAIN_OS_THREAD: AtomicUsize = AtomicUsize::new(0);

fn os_thread() -> usize {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn pthread_self() -> usize;
        }
        unsafe { pthread_self() }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
        }
        unsafe { GetCurrentThreadId() as usize }
    }
}

type Job = Box<dyn FnOnce() + Send>;

fn module_thread() -> Option<&'static mpsc::Sender<Job>> {
    static TX: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new().name("vst3-modules".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            }
        });
        spawned.ok().map(|_| tx)
    })
    .as_ref()
}

fn on_module_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static, timeout: Option<Duration>) -> Option<R> {
    let (rtx, rrx) = mpsc::sync_channel(1);
    module_thread()?
        .send(Box::new(move || {
            let _ = rtx.send(f());
        }))
        .ok()?;
    match timeout {
        Some(t) => rrx.recv_timeout(t).ok(),
        None => rrx.recv().ok(),
    }
}

type GetFactory = unsafe extern "system" fn() -> *mut sb::IPluginFactory;
type ModuleExit = unsafe extern "system" fn() -> bool;

#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, buf: *const u8, len: isize, is_dir: u8) -> *const c_void;
        pub fn CFBundleCreate(alloc: *const c_void, url: *const c_void) -> *mut c_void;
        pub fn CFRelease(cf: *const c_void);
    }
}

fn open_library(bundle: &Path, exe: &Path) -> Result<(libloading::Library, Option<ModuleExit>), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use libloading::os::unix::{Library as UnixLib, RTLD_LOCAL, RTLD_NOW};
        let lib = unsafe { UnixLib::open(Some(exe), RTLD_NOW | RTLD_LOCAL) }.map_err(|e| e.to_string())?;
        type ModuleEntry = unsafe extern "C" fn(*mut c_void) -> bool;
        let entry = unsafe { lib.get::<ModuleEntry>(b"ModuleEntry\0") }.ok().map(|s| *s);
        let raw = lib.into_raw();
        let lib = unsafe { UnixLib::from_raw(raw) };
        let mut exit = None;
        if let Some(f) = entry {
            if !unsafe { f(raw) } {
                return Err("ModuleEntry failed".into());
            }
            exit = unsafe { lib.get::<ModuleExit>(b"ModuleExit\0") }.ok().map(|s| *s);
        }
        let _ = bundle;
        Ok((lib.into(), exit))
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let lib = unsafe { libloading::Library::new(exe) }.map_err(|e| e.to_string())?;
        let mut exit: Option<ModuleExit> = None;
        #[cfg(windows)]
        {
            let _ = bundle;
            type InitDll = unsafe extern "system" fn() -> bool;
            if let Ok(f) = unsafe { lib.get::<InitDll>(b"InitDll\0") } {
                if !unsafe { f() } {
                    return Err("InitDll failed".into());
                }
                exit = unsafe { lib.get::<ModuleExit>(b"ExitDll\0") }.ok().map(|s| *s);
            }
        }
        #[cfg(target_os = "macos")]
        {
            type BundleEntry = unsafe extern "C" fn(*mut c_void) -> bool;
            let entry = unsafe { lib.get::<BundleEntry>(b"bundleEntry\0") }
                .or_else(|_| {
                    unsafe { lib.get::<BundleEntry>(b"BundleEntry\0") }
                })
                .ok()
                .map(|s| *s);
            if let Some(f) = entry
                && bundle.is_dir()
            {
                use std::os::unix::ffi::OsStrExt;
                let bytes = bundle.as_os_str().as_bytes();
                let len = isize::try_from(bytes.len()).map_err(|_| "path too long".to_string())?;
                let url = unsafe { cf::CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), len, 1) };
                if url.is_null() {
                    return Err("cannot make a CFURL for the bundle".into());
                }
                let cfbundle = unsafe { cf::CFBundleCreate(std::ptr::null(), url) };
                unsafe { cf::CFRelease(url) };
                if cfbundle.is_null() {
                    return Err("not a loadable bundle".into());
                }
                if !unsafe { f(cfbundle) } {
                    return Err("bundleEntry failed".into());
                }
                exit = unsafe { lib.get::<ModuleExit>(b"bundleExit\0") }
                    .or_else(|_| {
                        unsafe { lib.get::<ModuleExit>(b"BundleExit\0") }
                    })
                    .ok()
                    .map(|s| *s);
            }
        }
        Ok((lib, exit))
    }
}

pub(crate) fn exit_modules() {
    if MODULES_EXITED.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut cache = match loaded().try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let main = MAIN_OS_THREAD.load(Ordering::Acquire) == os_thread();
    let (mut kept, mut here, mut there) = (Vec::new(), Vec::new(), Vec::new());
    while let Some((path, b)) = cache.pop() {
        if b.leaked.load(Ordering::Acquire) || (b.entered_on == EntryThread::Main && !main) {
            kept.push((path, b));
            continue;
        }
        match Arc::try_unwrap(b) {
            Ok(bundle) if bundle.entered_on == EntryThread::Main => here.push(bundle),
            Ok(bundle) => there.push(bundle),
            Err(b) => kept.push((path, b)),
        }
    }
    kept.reverse();
    *cache = kept;
    drop(cache);
    here.into_iter().for_each(Bundle::exit);
    if !there.is_empty() {
        let _ = on_module_thread(move || there.into_iter().for_each(Bundle::exit), Some(Duration::from_secs(5)));
    }
}

fn register_exit_hook() {
    unsafe extern "C" {
        fn atexit(f: extern "C" fn()) -> c_int;
    }
    extern "C" fn hook() {
        let _ = std::panic::catch_unwind(exit_modules);
    }
    if unsafe { atexit(hook) } != 0 {
        log::warn!("vst3: cannot register the module exit handler");
    }
}

impl Bundle {
    pub fn load(path: &Path) -> Result<Arc<Bundle>, Vst3Error> {
        let mut cache = loaded().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, b)) = cache.iter().find(|(p, _)| p == path) {
            return Ok(b.clone());
        }
        let err = |m: &str| Vst3Error::Load(path.display().to_string(), m.to_string());
        if MODULES_EXITED.load(Ordering::Acquire) {
            return Err(err("plugin hosting has shut down"));
        }
        let b = if on_main_thread() {
            MAIN_OS_THREAD.store(os_thread(), Ordering::Release);
            Bundle::open(path, EntryThread::Main)?
        } else {
            let p = path.to_path_buf();
            on_module_thread(move || Bundle::open(&p, EntryThread::Modules), None).ok_or_else(|| err("the module thread is not running"))??
        };
        let b = Arc::new(b);
        cache.push((path.to_path_buf(), b.clone()));
        register_exit_hook();
        Ok(b)
    }

    fn open(path: &Path, entered_on: EntryThread) -> Result<Bundle, Vst3Error> {
        let err = |m: String| Vst3Error::Load(path.display().to_string(), m);
        let exe = scan::bundle_binary(path).ok_or_else(|| err("no loadable binary".into()))?;
        let (lib, exit) = open_library(path, &exe).map_err(err)?;
        let fail = |m: String| {
            if let Some(f) = exit {
                unsafe { f() };
            }
            err(m)
        };
        let get = match unsafe { lib.get::<GetFactory>(b"GetPluginFactory\0") } {
            Ok(sym) => *sym,
            Err(e) => return Err(fail(format!("not a VST3 plugin: {e}"))),
        };
        let raw = unsafe { get() };
        let factory = unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| fail("GetPluginFactory returned null".into()))?;
        Ok(Bundle { factory, exit, entered_on, leaked: AtomicBool::new(false), _lib: ManuallyDrop::new(lib) })
    }

    fn exit(self) {
        let Bundle { factory, exit, .. } = self;
        drop(factory);
        if let Some(f) = exit {
            unsafe { f() };
        }
    }

    pub fn mark_leaked(&self) {
        self.leaked.store(true, Ordering::Release);
    }

    fn factory_vendor(&self) -> String {
        let mut info: sb::PFactoryInfo = unsafe { std::mem::zeroed() };
        if ok(unsafe { self.factory.getFactoryInfo(&mut info) }) { read_c8(&info.vendor) } else { String::new() }
    }

    pub fn classes(&self) -> Vec<RawClass> {
        let n = unsafe { self.factory.countClasses() }.clamp(0, MAX_CLASSES);
        let f2 = self.factory.cast::<sb::IPluginFactory2>();
        let vendor = self.factory_vendor();
        let mut out = Vec::new();
        for i in 0..n {
            if let Some(f2) = &f2 {
                let mut c: sb::PClassInfo2 = unsafe { std::mem::zeroed() };
                if ok(unsafe { f2.getClassInfo2(i, &mut c) }) {
                    let v = read_c8(&c.vendor);
                    out.push(RawClass {
                        cid: tuid_bytes(&c.cid),
                        category: read_c8(&c.category),
                        name: read_c8(&c.name),
                        vendor: if v.is_empty() { vendor.clone() } else { v },
                        version: read_c8(&c.version),
                        sdk_version: read_c8(&c.sdkVersion),
                        sub_categories: read_c8(&c.subCategories),
                    });
                    continue;
                }
            }
            let mut c: sb::PClassInfo = unsafe { std::mem::zeroed() };
            if ok(unsafe { self.factory.getClassInfo(i, &mut c) }) {
                out.push(RawClass {
                    cid: tuid_bytes(&c.cid),
                    category: read_c8(&c.category),
                    name: read_c8(&c.name),
                    vendor: vendor.clone(),
                    version: String::new(),
                    sdk_version: String::new(),
                    sub_categories: String::new(),
                });
            }
        }
        out
    }

    fn create<I: Interface>(&self, cid: &[u8; 16]) -> Option<ComPtr<I>> {
        let mut obj: *mut c_void = std::ptr::null_mut();
        let iid = I::IID;
        let r = unsafe { self.factory.createInstance(cid.as_ptr().cast::<c_char>(), iid.as_ptr().cast::<c_char>(), &mut obj) };
        if !ok(r) {
            return None;
        }
        unsafe { ComPtr::from_raw(obj.cast::<I>()) }
    }

    pub fn instantiate(self: &Arc<Self>, cid: &[u8; 16]) -> Result<Instance, Vst3Error> {
        let name = hex(cid);
        let err = |m: &str| Vst3Error::Instantiate(name.clone(), m.to_string());
        let host = ComWrapper::new(HostContext::default());
        let host_unknown = host.as_com_ref::<sv::IHostApplication>().ok_or_else(|| err("host context"))?.upcast::<FUnknown>().as_ptr();
        let component = self.create::<sv::IComponent>(cid).ok_or_else(|| err("factory cannot create the component"))?;
        if !ok(unsafe { component.initialize(host_unknown) }) {
            return Err(err("component initialize failed"));
        }
        let mut inst = Instance {
            processor: None,
            controller: None,
            single: false,
            connections: None,
            gain_reduction: None,
            component_initialized: true,
            controller_initialized: false,
            active: false,
            processing: false,
            max_frames: 0,
            offline: false,
            in_params: ComWrapper::new(ParamChanges::new(PARAM_QUEUE_CAP)),
            out_params: ComWrapper::new(ParamChanges::new(PARAM_QUEUE_CAP)),
            in_events: ComWrapper::new(EventList::new(EVENT_CAP)),
            out_events: ComWrapper::new(EventList::new(EVENT_CAP)),
            context: unsafe { std::mem::zeroed() },
            event_in: false,
            editor: Arc::new(EditorLink::default()),
            component,
            host,
            bundle: self.clone(),
        };
        let processor = inst.component.cast::<sv::IAudioProcessor>().ok_or_else(|| err("no IAudioProcessor"))?;
        inst.processor = Some(processor);
        if let Some(c) = inst.component.cast::<sv::IEditController>() {
            inst.controller = Some(c);
            inst.single = true;
        } else {
            let mut ccid: sb::TUID = [0; 16];
            if ok(unsafe { inst.component.getControllerClassId(&mut ccid) })
                && let Some(c) = self.create::<sv::IEditController>(&tuid_bytes(&ccid))
            {
                if ok(unsafe { c.initialize(host_unknown) }) {
                    inst.controller = Some(c);
                    inst.controller_initialized = true;
                } else {
                    log::warn!("vst3 {name}: controller initialize failed; parameters unavailable");
                }
            }
        }
        if let Some(c) = &inst.controller {
            if !inst.single
                && let (Some(a), Some(b)) = (inst.component.cast::<sv::IConnectionPoint>(), c.cast::<sv::IConnectionPoint>())
            {
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                inst.connections = Some((a, b));
            }
            inst.sync_controller_state();
            if let Some(h) = inst.host.as_com_ref::<sv::IComponentHandler>() {
                unsafe { c.setComponentHandler(h.as_ptr()) };
            }
        }
        inst.gain_reduction = inst
            .controller
            .as_ref()
            .and_then(|c| GainReduction::query(c.as_ptr().cast()))
            .or_else(|| GainReduction::query(inst.component.as_ptr().cast()));
        inst.editor = Arc::new(EditorLink {
            outputs: Mutex::new(Vec::with_capacity(OUTPUTS_CAP)),
            inner: Mutex::new(EditorInner { component: Some(inst.component.clone()), controller: inst.controller.clone(), ..EditorInner::default() }),
        });
        Ok(inst)
    }
}


struct GainReduction(*mut sb::FUnknown);

#[repr(C)]
struct GainReductionVtbl {
    base: sb::FUnknownVtbl,
    get_gain_reduction_value_in_db: unsafe extern "system" fn(this: *mut sb::FUnknown) -> f64,
}

const GAIN_REDUCTION_IID: sb::TUID = vst3::uid(0x8e3c292c, 0x95924f9d, 0xb2590b1e, 0x100e4198);

impl GainReduction {
    fn query(obj: *mut sb::FUnknown) -> Option<GainReduction> {
        if obj.is_null() {
            return None;
        }
        let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
        let r = unsafe { ((*(*obj).vtbl).queryInterface)(obj, &GAIN_REDUCTION_IID, &mut out) };
        (r == sb::kResultOk && !out.is_null()).then(|| GainReduction(out.cast()))
    }

    fn db(&self) -> f32 {
        let v = unsafe { ((*((*self.0).vtbl as *const GainReductionVtbl)).get_gain_reduction_value_in_db)(self.0) };
        if v.is_finite() { (-v).clamp(0.0, 96.0) as f32 } else { 0.0 }
    }
}

impl Drop for GainReduction {
    fn drop(&mut self) {
        unsafe { ((*(*self.0).vtbl).release)(self.0) };
    }
}
fn tuid_bytes(t: &sb::TUID) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (o, b) in out.iter_mut().zip(t.iter()) {
        *o = *b as u8;
    }
    out
}

pub(crate) fn hex(cid: &[u8; 16]) -> String {
    cid.iter().map(|b| format!("{b:02X}")).collect()
}

pub(crate) struct HostContext {
    pub latency_changed: AtomicBool,
    pub restart_requested: AtomicBool,
    pub values_changed: AtomicBool,
    pub edits: Mutex<Vec<(u32, f64)>>,
}

impl Default for HostContext {
    fn default() -> Self {
        HostContext {
            latency_changed: AtomicBool::new(false),
            restart_requested: AtomicBool::new(false),
            values_changed: AtomicBool::new(false),
            edits: Mutex::new(Vec::with_capacity(EDIT_CAP)),
        }
    }
}

impl HostContext {
    pub fn take_edits(&self) -> Vec<(u32, f64)> {
        let mut e = self.edits.lock().unwrap_or_else(PoisonError::into_inner);
        let out = e.clone();
        e.clear();
        out
    }
}

impl Class for HostContext {
    type Interfaces = (sv::IHostApplication, sv::IComponentHandler);
}

impl IHostApplicationTrait for HostContext {
    unsafe fn getName(&self, name: *mut sv::String128) -> sb::tresult {
        match unsafe { name.as_mut() } {
            Some(n) => {
                write_c16("OpenPT", n);
                sb::kResultOk
            }
            None => sb::kInvalidArgument,
        }
    }

    unsafe fn createInstance(&self, cid: *mut sb::TUID, iid: *mut sb::TUID, obj: *mut *mut c_void) -> sb::tresult {
        let Some(o) = (unsafe { obj.as_mut() }) else { return sb::kInvalidArgument };
        *o = std::ptr::null_mut();
        let (cid, iid) = unsafe { (tuid_from_ptr(cid), tuid_from_ptr(iid)) };
        let made = match (cid, iid) {
            (Some(c), Some(i)) if c == sv::IMessage::IID && i == sv::IMessage::IID => {
                ComWrapper::new(HostMessage::default()).to_com_ptr::<sv::IMessage>().map(|p| p.into_raw().cast::<c_void>())
            }
            (Some(c), Some(i)) if c == sv::IAttributeList::IID && i == sv::IAttributeList::IID => {
                ComWrapper::new(HostAttributes::default()).to_com_ptr::<sv::IAttributeList>().map(|p| p.into_raw().cast::<c_void>())
            }
            _ => None,
        };
        match made {
            Some(p) => {
                *o = p;
                sb::kResultOk
            }
            None => sb::kNoInterface,
        }
    }
}

unsafe fn tuid_from_ptr(t: *const sb::TUID) -> Option<[u8; 16]> {
    let t = unsafe { t.as_ref() }?;
    let mut out = [0u8; 16];
    for (o, b) in out.iter_mut().zip(t.iter()) {
        *o = *b as u8;
    }
    Some(out)
}

const MAX_ATTRS: usize = 256;
const MAX_ATTR_BYTES: usize = 16 << 20;

enum Attr {
    Int(i64),
    Float(f64),
    Str(Vec<u16>),
    Bin(Vec<u8>),
}

#[derive(Default)]
struct HostAttributes {
    map: Mutex<std::collections::HashMap<Vec<u8>, Attr>>,
}

impl Class for HostAttributes {
    type Interfaces = (sv::IAttributeList,);
}

unsafe fn attr_key(id: sv::IAttributeList_::AttrID) -> Option<Vec<u8>> {
    if id.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(id) };
    let b = s.to_bytes();
    (b.len() <= 256).then(|| b.to_vec())
}

impl HostAttributes {
    fn set(&self, id: sv::IAttributeList_::AttrID, v: Attr) -> sb::tresult {
        let Some(k) = (unsafe { attr_key(id) }) else { return sb::kInvalidArgument };
        let mut m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        if m.len() >= MAX_ATTRS && !m.contains_key(&k) {
            return sb::kOutOfMemory;
        }
        m.insert(k, v);
        sb::kResultOk
    }
}

impl sv::IAttributeListTrait for HostAttributes {
    unsafe fn setInt(&self, id: sv::IAttributeList_::AttrID, value: i64) -> sb::tresult {
        self.set(id, Attr::Int(value))
    }

    unsafe fn getInt(&self, id: sv::IAttributeList_::AttrID, value: *mut i64) -> sb::tresult {
        let (Some(k), Some(out)) = (unsafe { attr_key(id) }, unsafe { value.as_mut() }) else { return sb::kInvalidArgument };
        match self.map.lock().unwrap_or_else(PoisonError::into_inner).get(&k) {
            Some(Attr::Int(v)) => {
                *out = *v;
                sb::kResultOk
            }
            _ => sb::kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: sv::IAttributeList_::AttrID, value: f64) -> sb::tresult {
        self.set(id, Attr::Float(value))
    }

    unsafe fn getFloat(&self, id: sv::IAttributeList_::AttrID, value: *mut f64) -> sb::tresult {
        let (Some(k), Some(out)) = (unsafe { attr_key(id) }, unsafe { value.as_mut() }) else { return sb::kInvalidArgument };
        match self.map.lock().unwrap_or_else(PoisonError::into_inner).get(&k) {
            Some(Attr::Float(v)) => {
                *out = *v;
                sb::kResultOk
            }
            _ => sb::kResultFalse,
        }
    }

    unsafe fn setString(&self, id: sv::IAttributeList_::AttrID, string: *const sv::TChar) -> sb::tresult {
        if string.is_null() {
            return sb::kInvalidArgument;
        }
        let mut v = Vec::new();
        loop {
            let c = unsafe { *string.add(v.len()) };
            if c == 0 {
                break;
            }
            if v.len() >= MAX_ATTR_BYTES / 2 {
                return sb::kOutOfMemory;
            }
            v.push(c);
        }
        v.push(0);
        self.set(id, Attr::Str(v))
    }

    unsafe fn getString(&self, id: sv::IAttributeList_::AttrID, string: *mut sv::TChar, size_in_bytes: u32) -> sb::tresult {
        let Some(k) = (unsafe { attr_key(id) }) else { return sb::kInvalidArgument };
        let cap = size_in_bytes as usize / 2;
        if string.is_null() || cap == 0 {
            return sb::kInvalidArgument;
        }
        match self.map.lock().unwrap_or_else(PoisonError::into_inner).get(&k) {
            Some(Attr::Str(v)) => {
                let n = v.len().min(cap);
                unsafe {
                    std::ptr::copy_nonoverlapping(v.as_ptr(), string, n);
                    *string.add(n.saturating_sub(1)) = 0;
                }
                sb::kResultOk
            }
            _ => sb::kResultFalse,
        }
    }

    unsafe fn setBinary(&self, id: sv::IAttributeList_::AttrID, data: *const c_void, size_in_bytes: u32) -> sb::tresult {
        let n = size_in_bytes as usize;
        if n > MAX_ATTR_BYTES {
            return sb::kOutOfMemory;
        }
        let v = if data.is_null() || n == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), n) }.to_vec()
        };
        self.set(id, Attr::Bin(v))
    }

    unsafe fn getBinary(&self, id: sv::IAttributeList_::AttrID, data: *mut *const c_void, size_in_bytes: *mut u32) -> sb::tresult {
        let (Some(k), Some(d), Some(sz)) = (unsafe { attr_key(id) }, unsafe { data.as_mut() }, unsafe { size_in_bytes.as_mut() }) else {
            return sb::kInvalidArgument;
        };
        match self.map.lock().unwrap_or_else(PoisonError::into_inner).get(&k) {
            Some(Attr::Bin(v)) => {
                *d = v.as_ptr().cast::<c_void>();
                *sz = v.len() as u32;
                sb::kResultOk
            }
            _ => sb::kResultFalse,
        }
    }
}

#[derive(Default)]
struct HostMessage {
    id: Mutex<Option<std::ffi::CString>>,
    attrs: OnceLock<ComWrapper<HostAttributes>>,
}

impl Class for HostMessage {
    type Interfaces = (sv::IMessage,);
}

impl sv::IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> sb::FIDString {
        self.id.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
    }

    unsafe fn setMessageID(&self, id: sb::FIDString) {
        let v = if id.is_null() {
            None
        } else {
            Some(unsafe { std::ffi::CStr::from_ptr(id) }.to_owned())
        };
        *self.id.lock().unwrap_or_else(PoisonError::into_inner) = v;
    }

    unsafe fn getAttributes(&self) -> *mut sv::IAttributeList {
        self.attrs
            .get_or_init(|| ComWrapper::new(HostAttributes::default()))
            .as_com_ref::<sv::IAttributeList>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

impl IComponentHandlerTrait for HostContext {
    unsafe fn beginEdit(&self, _id: sv::ParamID) -> sb::tresult {
        sb::kResultOk
    }
    unsafe fn performEdit(&self, id: sv::ParamID, value: sv::ParamValue) -> sb::tresult {
        if value.is_finite() {
            let mut e = self.edits.lock().unwrap_or_else(PoisonError::into_inner);
            let v = value.clamp(0.0, 1.0);
            if let Some(x) = e.iter_mut().find(|(i, _)| *i == id) {
                x.1 = v;
            } else if e.len() < EDIT_CAP {
                e.push((id, v));
            }
            drop(e);
            crate::wake_ui();
        }
        sb::kResultOk
    }
    unsafe fn endEdit(&self, _id: sv::ParamID) -> sb::tresult {
        sb::kResultOk
    }
    unsafe fn restartComponent(&self, flags: sb::int32) -> sb::tresult {
        if flags as u32 & sv::RestartFlags_::kLatencyChanged as u32 != 0 {
            self.latency_changed.store(true, Ordering::Relaxed);
        }
        if flags as u32 & (sv::RestartFlags_::kParamValuesChanged as u32 | sv::RestartFlags_::kReloadComponent as u32) != 0 {
            self.values_changed.store(true, Ordering::Relaxed);
        }
        self.restart_requested.store(true, Ordering::Relaxed);
        sb::kResultOk
    }
}

#[derive(Default)]
pub(crate) struct ParamQueue {
    id: AtomicU32,
    offset: AtomicI32,
    value: AtomicU64,
    points: AtomicI32,
}

impl Class for ParamQueue {
    type Interfaces = (sv::IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> sv::ParamID {
        self.id.load(Ordering::Relaxed)
    }
    unsafe fn getPointCount(&self) -> sb::int32 {
        self.points.load(Ordering::Relaxed)
    }
    unsafe fn getPoint(&self, index: sb::int32, offset: *mut sb::int32, value: *mut sv::ParamValue) -> sb::tresult {
        if index != 0 || self.points.load(Ordering::Relaxed) < 1 {
            return sb::kInvalidArgument;
        }
        if let (Some(o), Some(v)) = (unsafe { offset.as_mut() }, unsafe { value.as_mut() }) {
            *o = self.offset.load(Ordering::Relaxed);
            *v = f64::from_bits(self.value.load(Ordering::Relaxed));
            sb::kResultOk
        } else {
            sb::kInvalidArgument
        }
    }
    unsafe fn addPoint(&self, offset: sb::int32, value: sv::ParamValue, index: *mut sb::int32) -> sb::tresult {
        self.offset.store(offset, Ordering::Relaxed);
        self.value.store(value.to_bits(), Ordering::Relaxed);
        self.points.store(1, Ordering::Relaxed);
        if let Some(i) = unsafe { index.as_mut() } {
            *i = 0;
        }
        sb::kResultOk
    }
}

pub(crate) struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    count: AtomicI32,
}

impl Class for ParamChanges {
    type Interfaces = (sv::IParameterChanges,);
}

impl ParamChanges {
    fn new(cap: usize) -> ParamChanges {
        ParamChanges { queues: (0..cap).map(|_| ComWrapper::new(ParamQueue::default())).collect(), count: AtomicI32::new(0) }
    }

    fn clear(&self) {
        self.count.store(0, Ordering::Relaxed);
    }

    fn queue_for(&self, id: u32) -> Option<(i32, &ParamQueue)> {
        let n = self.count.load(Ordering::Relaxed).max(0);
        for (i, q) in self.queues.iter().enumerate().take(n as usize) {
            if q.id.load(Ordering::Relaxed) == id {
                return Some((i32::try_from(i).ok()?, q));
            }
        }
        let q = self.queues.get(n as usize)?;
        q.id.store(id, Ordering::Relaxed);
        q.points.store(0, Ordering::Relaxed);
        self.count.store(n + 1, Ordering::Relaxed);
        Some((n, q))
    }

    fn set(&self, id: u32, offset: i32, value: f64) {
        if let Some((_, q)) = self.queue_for(id) {
            q.offset.store(offset, Ordering::Relaxed);
            q.value.store(value.to_bits(), Ordering::Relaxed);
            q.points.store(1, Ordering::Relaxed);
        }
    }

    fn queue_ptr(&self, index: usize) -> *mut sv::IParamValueQueue {
        self.queues.get(index).and_then(|q| q.as_com_ref::<sv::IParamValueQueue>()).map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> sb::int32 {
        self.count.load(Ordering::Relaxed)
    }
    unsafe fn getParameterData(&self, index: sb::int32) -> *mut sv::IParamValueQueue {
        if index < 0 || index >= self.count.load(Ordering::Relaxed) {
            return std::ptr::null_mut();
        }
        self.queue_ptr(index as usize)
    }
    unsafe fn addParameterData(&self, id: *const sv::ParamID, index: *mut sb::int32) -> *mut sv::IParamValueQueue {
        let Some(&id) = (unsafe { id.as_ref() }) else { return std::ptr::null_mut() };
        let Some((i, _)) = self.queue_for(id) else { return std::ptr::null_mut() };
        if let Some(out) = unsafe { index.as_mut() } {
            *out = i;
        }
        self.queue_ptr(i as usize)
    }
}

pub(crate) struct EventList {
    events: Mutex<Vec<sv::Event>>,
    cap: usize,
}

impl Class for EventList {
    type Interfaces = (sv::IEventList,);
}

impl EventList {
    fn new(cap: usize) -> EventList {
        EventList { events: Mutex::new(Vec::with_capacity(cap)), cap }
    }
    fn with<R>(&self, f: impl FnOnce(&mut Vec<sv::Event>) -> R) -> Option<R> {
        match self.events.try_lock() {
            Ok(mut g) => Some(f(&mut g)),
            Err(std::sync::TryLockError::Poisoned(p)) => Some(f(&mut p.into_inner())),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> sb::int32 {
        self.with(|v| i32::try_from(v.len()).unwrap_or(i32::MAX)).unwrap_or(0)
    }
    unsafe fn getEvent(&self, index: sb::int32, e: *mut sv::Event) -> sb::tresult {
        let Some(out) = (unsafe { e.as_mut() }) else { return sb::kInvalidArgument };
        let Ok(i) = usize::try_from(index) else { return sb::kInvalidArgument };
        match self.with(|v| v.get(i).copied()).flatten() {
            Some(ev) => {
                *out = ev;
                sb::kResultOk
            }
            None => sb::kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, e: *mut sv::Event) -> sb::tresult {
        let Some(&ev) = (unsafe { e.as_ref() }) else { return sb::kInvalidArgument };
        let cap = self.cap;
        match self.with(|v| {
            if v.len() < cap {
                v.push(ev);
                true
            } else {
                false
            }
        }) {
            Some(true) => sb::kResultOk,
            _ => sb::kResultFalse,
        }
    }
}

pub(crate) struct MemoryStream {
    inner: Mutex<(Vec<u8>, usize)>,
}

impl Class for MemoryStream {
    type Interfaces = (sb::IBStream,);
}

impl MemoryStream {
    fn new(data: Vec<u8>) -> ComWrapper<MemoryStream> {
        ComWrapper::new(MemoryStream { inner: Mutex::new((data, 0)) })
    }

    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.lock().0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<u8>, usize)> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: sb::int32, num_read: *mut sb::int32) -> sb::tresult {
        let mut g = self.lock();
        let (data, pos) = &mut *g;
        let want = usize::try_from(num_bytes).unwrap_or(0);
        let avail = data.len().saturating_sub(*pos);
        let n = want.min(avail);
        if n > 0 && !buffer.is_null() {
            let Some(src) = data.get(*pos..*pos + n) else { return sb::kInternalError };
            unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), buffer.cast::<u8>(), n) };
            *pos += n;
        }
        if let Some(r) = unsafe { num_read.as_mut() } {
            *r = i32::try_from(n).unwrap_or(0);
        }
        sb::kResultOk
    }
    unsafe fn write(&self, buffer: *mut c_void, num_bytes: sb::int32, num_written: *mut sb::int32) -> sb::tresult {
        let mut g = self.lock();
        let (data, pos) = &mut *g;
        let n = usize::try_from(num_bytes).unwrap_or(0);
        let end = pos.saturating_add(n);
        if buffer.is_null() || end > MAX_STATE_BYTES {
            return sb::kResultFalse;
        }
        if data.len() < end {
            data.resize(end, 0);
        }
        let src = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) };
        if let Some(dst) = data.get_mut(*pos..end) {
            dst.copy_from_slice(src);
        }
        *pos = end;
        if let Some(w) = unsafe { num_written.as_mut() } {
            *w = num_bytes;
        }
        sb::kResultOk
    }
    unsafe fn seek(&self, pos: sb::int64, mode: sb::int32, result: *mut sb::int64) -> sb::tresult {
        let mut g = self.lock();
        let len = i64::try_from(g.0.len()).unwrap_or(i64::MAX);
        let cur = i64::try_from(g.1).unwrap_or(0);
        let base = match mode as u32 {
            x if x == sb::IBStream_::IStreamSeekMode_::kIBSeekSet as u32 => 0,
            x if x == sb::IBStream_::IStreamSeekMode_::kIBSeekCur as u32 => cur,
            x if x == sb::IBStream_::IStreamSeekMode_::kIBSeekEnd as u32 => len,
            _ => return sb::kInvalidArgument,
        };
        let new = base.saturating_add(pos).clamp(0, len);
        g.1 = usize::try_from(new).unwrap_or(0);
        if let Some(r) = unsafe { result.as_mut() } {
            *r = new;
        }
        sb::kResultOk
    }
    unsafe fn tell(&self, pos: *mut sb::int64) -> sb::tresult {
        let g = self.lock();
        match unsafe { pos.as_mut() } {
            Some(p) => {
                *p = i64::try_from(g.1).unwrap_or(0);
                sb::kResultOk
            }
            None => sb::kInvalidArgument,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NoteEvent {
    On { time: u32, channel: u8, key: u8, velocity: f32 },
    Off { time: u32, channel: u8, key: u8 },
    Pressure { time: u32, channel: u8, key: u8, pressure: f32 },
}

impl NoteEvent {
    pub fn time(&self) -> u32 {
        match *self {
            NoteEvent::On { time, .. } | NoteEvent::Off { time, .. } | NoteEvent::Pressure { time, .. } => time,
        }
    }
    pub fn set_time(&mut self, t: u32) {
        match self {
            NoteEvent::On { time, .. } | NoteEvent::Off { time, .. } | NoteEvent::Pressure { time, .. } => *time = t,
        }
    }

    fn to_raw(self, offset: u32) -> sv::Event {
        let mut e: sv::Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = i32::try_from(self.time().saturating_sub(offset)).unwrap_or(0);
        e.flags = sv::Event_::EventFlags_::kIsLive as u16;
        match self {
            NoteEvent::On { channel, key, velocity, .. } => {
                e.r#type = sv::Event_::EventTypes_::kNoteOnEvent as u16;
                e.__field0.noteOn =
                    sv::NoteOnEvent { channel: i16::from(channel & 0x0F), pitch: i16::from(key), tuning: 0.0, velocity, length: 0, noteId: -1 };
            }
            NoteEvent::Off { channel, key, .. } => {
                e.r#type = sv::Event_::EventTypes_::kNoteOffEvent as u16;
                e.__field0.noteOff =
                    sv::NoteOffEvent { channel: i16::from(channel & 0x0F), pitch: i16::from(key), velocity: 0.0, noteId: -1, tuning: 0.0 };
            }
            NoteEvent::Pressure { channel, key, pressure, .. } => {
                e.r#type = sv::Event_::EventTypes_::kPolyPressureEvent as u16;
                e.__field0.polyPressure = sv::PolyPressureEvent { channel: i16::from(channel & 0x0F), pitch: i16::from(key), pressure, noteId: -1 };
            }
        }
        e
    }
}

pub(crate) struct BusBuffers {
    data: Vec<Vec<Vec<f32>>>,
    ptrs: Vec<Vec<*mut f32>>,
    bufs: Vec<sv::AudioBusBuffers>,
    frames: usize,
}

unsafe impl Send for BusBuffers {}

impl BusBuffers {
    pub fn new(channels: &[u32], frames: usize) -> BusBuffers {
        let data: Vec<Vec<Vec<f32>>> =
            channels.iter().take(MAX_BUSES as usize).map(|&c| (0..c.min(MAX_BUS_CHANNELS)).map(|_| vec![0.0; frames]).collect()).collect();
        let ptrs = data.iter().map(|p| vec![std::ptr::null_mut(); p.len()]).collect();
        let bufs = data
            .iter()
            .map(|_| sv::AudioBusBuffers {
                numChannels: 0,
                silenceFlags: 0,
                __field0: sv::AudioBusBuffers__type0 { channelBuffers32: std::ptr::null_mut() },
            })
            .collect();
        BusBuffers { data, ptrs, bufs, frames }
    }

    pub fn channels(&self, bus: usize) -> usize {
        self.data.get(bus).map_or(0, Vec::len)
    }

    pub fn channel(&self, bus: usize, ch: usize) -> Option<&[f32]> {
        self.data.get(bus).and_then(|p| p.get(ch)).map(Vec::as_slice)
    }

    pub fn channel_mut(&mut self, bus: usize, ch: usize) -> Option<&mut [f32]> {
        self.data.get_mut(bus).and_then(|p| p.get_mut(ch)).map(Vec::as_mut_slice)
    }

    fn refresh(&mut self) {
        for ((d, p), b) in self.data.iter_mut().zip(self.ptrs.iter_mut()).zip(self.bufs.iter_mut()) {
            for (dc, pc) in d.iter_mut().zip(p.iter_mut()) {
                *pc = dc.as_mut_ptr();
            }
            b.numChannels = i32::try_from(p.len()).unwrap_or(0);
            b.silenceFlags = 0;
            b.__field0.channelBuffers32 = p.as_mut_ptr();
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RawParam {
    pub id: u32,
    pub title: String,
    pub units: String,
    pub step_count: i32,
    pub default_normalized: f64,
    pub flags: i32,
}

pub(crate) const PARAM_HIDDEN: i32 = sv::ParameterInfo_::ParameterFlags_::kIsHidden;
pub(crate) const PARAM_READ_ONLY: i32 = sv::ParameterInfo_::ParameterFlags_::kIsReadOnly;
pub(crate) const PARAM_BYPASS: i32 = sv::ParameterInfo_::ParameterFlags_::kIsBypass;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Layout {
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
    pub event_input: bool,
    pub key: bool,
}

fn apply_transport(c: &mut sv::ProcessContext, t: &Transport) {
    use sv::ProcessContext_::StatesAndFlags_::*;
    let mut state = kTempoValid | kTimeSigValid | kProjectTimeMusicValid | kBarPositionValid | kContTimeValid;
    if t.playing {
        state |= kPlaying;
    }
    if t.recording {
        state |= kRecording;
    }
    #[allow(clippy::unnecessary_cast)]
    let state = state as u32;
    c.state = state;
    c.tempo = t.tempo;
    c.timeSigNumerator = i32::try_from(t.numerator).unwrap_or(4);
    c.timeSigDenominator = i32::try_from(t.denominator).unwrap_or(4);
    c.projectTimeSamples = t.samples;
    c.projectTimeMusic = t.quarters;
    c.barPositionMusic = t.bar_start;
}

pub(crate) struct Instance {
    processor: Option<ComPtr<sv::IAudioProcessor>>,
    controller: Option<ComPtr<sv::IEditController>>,
    single: bool,
    connections: Option<(ComPtr<sv::IConnectionPoint>, ComPtr<sv::IConnectionPoint>)>,
    gain_reduction: Option<GainReduction>,
    component_initialized: bool,
    controller_initialized: bool,
    active: bool,
    processing: bool,
    max_frames: usize,
    offline: bool,
    in_params: ComWrapper<ParamChanges>,
    out_params: ComWrapper<ParamChanges>,
    in_events: ComWrapper<EventList>,
    out_events: ComWrapper<EventList>,
    context: sv::ProcessContext,
    event_in: bool,
    editor: Arc<EditorLink>,
    component: ComPtr<sv::IComponent>,
    host: ComWrapper<HostContext>,
    bundle: Arc<Bundle>,
}

unsafe impl Send for Instance {}

impl Instance {
    fn processor(&self) -> Option<&ComPtr<sv::IAudioProcessor>> {
        self.processor.as_ref()
    }

    pub fn set_offline(&mut self, on: bool) {
        self.offline = on;
    }

    fn process_mode(&self) -> i32 {
        if self.offline { sv::ProcessModes_::kOffline as i32 } else { sv::ProcessModes_::kRealtime as i32 }
    }

    pub fn host(&self) -> &HostContext {
        &self.host
    }

    pub fn host_ref(&self) -> ComWrapper<HostContext> {
        self.host.clone()
    }

    pub fn editor(&self) -> Option<Arc<EditorLink>> {
        self.editor.has_controller().then(|| Arc::clone(&self.editor))
    }

    pub fn state_link(&self) -> Arc<EditorLink> {
        Arc::clone(&self.editor)
    }

    fn read_state(f: impl FnOnce(*mut sb::IBStream) -> sb::tresult) -> Option<Vec<u8>> {
        let stream = MemoryStream::new(Vec::new());
        let s = stream.as_com_ref::<sb::IBStream>()?;
        ok(f(s.as_ptr())).then(|| stream.take())
    }

    pub fn load_state(&self, data: &[u8]) -> bool {
        let Some((comp, ctrl)) = split_state(data) else { return false };
        let stream = MemoryStream::new(comp.to_vec());
        let Some(s) = stream.as_com_ref::<sb::IBStream>() else { return false };
        if !ok(unsafe { self.component.setState(s.as_ptr()) }) {
            return false;
        }
        if let Some(c) = &self.controller {
            stream.lock().1 = 0;
            unsafe { c.setComponentState(s.as_ptr()) };
            if !ctrl.is_empty() {
                let cs = MemoryStream::new(ctrl.to_vec());
                if let Some(r) = cs.as_com_ref::<sb::IBStream>() {
                    unsafe { c.setState(r.as_ptr()) };
                }
            }
        }
        true
    }

    fn sync_controller_state(&self) {
        let Some(c) = &self.controller else { return };
        let stream = ComWrapper::new(MemoryStream { inner: Mutex::new((Vec::new(), 0)) });
        let Some(s) = stream.as_com_ref::<sb::IBStream>() else { return };
        if ok(unsafe { self.component.getState(s.as_ptr()) }) {
            stream.lock().1 = 0;
            unsafe { c.setComponentState(s.as_ptr()) };
        }
    }

    pub fn params(&self) -> Vec<RawParam> {
        let Some(c) = &self.controller else { return Vec::new() };
        let n = unsafe { c.getParameterCount() }.clamp(0, MAX_PARAMS);
        let mut out = Vec::new();
        for i in 0..n {
            let mut info: sv::ParameterInfo = unsafe { std::mem::zeroed() };
            if !ok(unsafe { c.getParameterInfo(i, &mut info) }) {
                continue;
            }
            let d = info.defaultNormalizedValue;
            out.push(RawParam {
                id: info.id,
                title: read_c16(&info.title),
                units: read_c16(&info.units),
                step_count: info.stepCount.max(0),
                default_normalized: if d.is_finite() { d.clamp(0.0, 1.0) } else { 0.0 },
                flags: info.flags,
            });
        }
        out
    }

    pub fn to_plain(&self, id: u32, normalized: f64) -> Option<f64> {
        let c = self.controller.as_ref()?;
        let v = unsafe { c.normalizedParamToPlain(id, normalized.clamp(0.0, 1.0)) };
        v.is_finite().then_some(v)
    }

    pub fn to_normalized(&self, id: u32, plain: f64) -> Option<f64> {
        let c = self.controller.as_ref()?;
        if !plain.is_finite() {
            return None;
        }
        let v = unsafe { c.plainParamToNormalized(id, plain) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    pub fn normalized_value(&self, id: u32) -> Option<f64> {
        let c = self.controller.as_ref()?;
        let v = unsafe { c.getParamNormalized(id) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    pub fn midi_mapping(&self) -> Vec<Option<u32>> {
        let Some(m) = self.controller.as_ref().and_then(|c| c.cast::<sv::IMidiMapping>()) else { return Vec::new() };
        (0..=PITCH_BEND)
            .map(|n| {
                let mut id: sv::ParamID = 0;
                let n = i16::try_from(n).unwrap_or(0);
                ok(unsafe { m.getMidiControllerAssignment(0, 0, n, &mut id) }).then_some(id)
            })
            .collect()
    }

    pub fn value_text(&self, id: u32, normalized: f64) -> Option<String> {
        let c = self.controller.as_ref()?;
        let mut s: sv::String128 = [0; 128];
        ok(unsafe { c.getParamStringByValue(id, normalized.clamp(0.0, 1.0), &mut s) }).then(|| read_c16(&s)).filter(|t| !t.is_empty())
    }

    #[cfg(test)]
    pub fn reports_gain_reduction(&self) -> bool {
        self.gain_reduction.is_some()
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction.as_ref().map_or(0.0, GainReduction::db)
    }

    pub fn latency(&self) -> u32 {
        self.processor().map_or(0, |p| unsafe { p.getLatencySamples() })
    }

    pub fn tail(&self) -> u32 {
        self.processor().map_or(0, |p| unsafe { p.getTailSamples() })
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    fn bus_count(&self, media: sv::MediaTypes, dir: sv::BusDirections) -> i32 {
        unsafe { self.component.getBusCount(media as sv::MediaType, dir as sv::BusDirection) }.clamp(0, MAX_BUSES)
    }

    fn arrangement(&self, dir: sv::BusDirections, index: i32) -> Option<sv::SpeakerArrangement> {
        let p = self.processor()?;
        let mut arr: sv::SpeakerArrangement = 0;
        ok(unsafe { p.getBusArrangement(dir as sv::BusDirection, index, &mut arr) }).then_some(arr)
    }

    fn bus_type(&self, dir: sv::BusDirections, index: i32) -> Option<i32> {
        let mut info: sv::BusInfo = unsafe { std::mem::zeroed() };
        ok(unsafe { self.component.getBusInfo(sv::MediaTypes_::kAudio as sv::MediaType, dir as sv::BusDirection, index, &mut info) })
            .then_some(info.busType)
    }

    fn bus_channels(&self, dir: sv::BusDirections, index: i32) -> u32 {
        if let Some(arr) = self.arrangement(dir, index) {
            return arr.count_ones().min(MAX_BUS_CHANNELS);
        }
        let mut info: sv::BusInfo = unsafe { std::mem::zeroed() };
        if ok(unsafe { self.component.getBusInfo(sv::MediaTypes_::kAudio as sv::MediaType, dir as sv::BusDirection, index, &mut info) }) {
            u32::try_from(info.channelCount).unwrap_or(0).min(MAX_BUS_CHANNELS)
        } else {
            0
        }
    }

    pub fn midi_out(&self, out: &mut Vec<(usize, [u8; 3])>) {
        use sv::Event_::EventTypes_ as T;
        let byte = |v: i32| v.clamp(0, 127) as u8;
        let vel = |v: f32| (v * 127.0).round().clamp(0.0, 127.0) as u8;
        self.out_events.with(|v| {
            for e in v.iter() {
                let at = usize::try_from(e.sampleOffset).unwrap_or(0);
                let msg = unsafe {
                    match u32::from(e.r#type) {
                        t if t == T::kNoteOnEvent as u32 => {
                            let n = e.__field0.noteOn;
                            let v = vel(n.velocity);
                            [if v == 0 { 0x80 } else { 0x90 } | (n.channel as u8 & 0x0F), byte(i32::from(n.pitch)), v]
                        }
                        t if t == T::kNoteOffEvent as u32 => {
                            let n = e.__field0.noteOff;
                            [0x80 | (n.channel as u8 & 0x0F), byte(i32::from(n.pitch)), vel(n.velocity)]
                        }
                        t if t == T::kPolyPressureEvent as u32 => {
                            let n = e.__field0.polyPressure;
                            [0xA0 | (n.channel as u8 & 0x0F), byte(i32::from(n.pitch)), vel(n.pressure)]
                        }
                        t if t == T::kLegacyMIDICCOutEvent as u32 => {
                            let c = e.__field0.midiCCOut;
                            let ch = c.channel as u8 & 0x0F;
                            let (a, b) = (byte(i32::from(c.value)), byte(i32::from(c.value2)));
                            match c.controlNumber {
                                n @ 0..=127 => [0xB0 | ch, n, a],
                                128 => [0xD0 | ch, a, 0],
                                129 => [0xE0 | ch, a, b],
                                130 => [0xC0 | ch, a, 0],
                                _ => continue,
                            }
                        }
                        _ => continue,
                    }
                };
                if out.len() < out.capacity() {
                    out.push((at, msg));
                }
            }
        });
    }

    pub fn configure(&mut self, sample_rate: f64, max_frames: usize, mono: bool, all_outputs: bool) -> Result<Layout, Vst3Error> {
        self.shutdown();
        let audio = sv::MediaTypes_::kAudio;
        let event = sv::MediaTypes_::kEvent;
        let (din, dout) = (sv::BusDirections_::kInput, sv::BusDirections_::kOutput);
        let nin = self.bus_count(audio, din);
        let nout = self.bus_count(audio, dout);
        let current = |s: &Self, dir: sv::BusDirections, n: i32| -> Vec<sv::SpeakerArrangement> {
            (0..n).map(|i| s.arrangement(dir, i).unwrap_or(sv::SpeakerArr::kStereo)).collect()
        };
        let want = if mono { sv::SpeakerArr::kMono } else { sv::SpeakerArr::kStereo };
        let p = self.processor().ok_or_else(|| Vst3Error::Process("no audio processor".into()))?.clone();
        let mut tried_ok = false;
        for main in [want, sv::SpeakerArr::kStereo] {
            let mut ins = current(self, din, nin);
            let mut outs = current(self, dout, nout);
            if let Some(a) = ins.first_mut() {
                *a = main;
            }
            if let Some(a) = outs.first_mut() {
                *a = main;
            }
            if ok(unsafe { p.setBusArrangements(ins.as_mut_ptr(), nin, outs.as_mut_ptr(), nout) }) {
                tried_ok = true;
                break;
            }
            if main == sv::SpeakerArr::kStereo {
                break;
            }
        }
        if !tried_ok {
            log::debug!("vst3: plugin refused mono/stereo bus arrangements; using its own");
        }
        let key = nin > 1 && self.bus_type(din, 1) == Some(sv::BusTypes_::kAux as i32) && self.bus_channels(din, 1) > 0;
        let layout = Layout {
            inputs: (0..nin).map(|i| self.bus_channels(din, i)).collect(),
            outputs: (0..nout).map(|i| self.bus_channels(dout, i)).collect(),
            event_input: self.bus_count(event, din) > 0,
            key,
        };
        for (dir, n) in [(din, nin), (dout, nout)] {
            for i in 0..n {
                let on = i == 0 || (key && dir == din && i == 1) || (all_outputs && dir == dout);
                unsafe { self.component.activateBus(audio as sv::MediaType, dir as sv::BusDirection, i, u8::from(on)) };
            }
        }
        if layout.event_input {
            unsafe { self.component.activateBus(event as sv::MediaType, din as sv::BusDirection, 0, 1) };
        }
        if self.bus_count(event, dout) > 0 {
            unsafe { self.component.activateBus(event as sv::MediaType, dout as sv::BusDirection, 0, 1) };
        }
        self.event_in = layout.event_input;
        let sample32 = sv::SymbolicSampleSizes_::kSample32 as i32;
        if !ok(unsafe { p.canProcessSampleSize(sample32) }) {
            return Err(Vst3Error::Process("plugin cannot process 32-bit float".into()));
        }
        let max = i32::try_from(max_frames).unwrap_or(i32::MAX);
        let mut setup = sv::ProcessSetup {
            processMode: self.process_mode(),
            symbolicSampleSize: sample32,
            maxSamplesPerBlock: max,
            sampleRate: sample_rate,
        };
        if !ok(unsafe { p.setupProcessing(&mut setup) }) {
            return Err(Vst3Error::Process("setupProcessing failed".into()));
        }
        if !ok(unsafe { self.component.setActive(1) }) {
            return Err(Vst3Error::Process("setActive failed".into()));
        }
        self.active = true;
        self.max_frames = max_frames;
        self.context.sampleRate = sample_rate;
        self.context.tempo = 120.0;
        self.context.timeSigNumerator = 4;
        self.context.timeSigDenominator = 4;
        #[allow(clippy::unnecessary_cast)]
        let flags = (sv::ProcessContext_::StatesAndFlags_::kTempoValid | sv::ProcessContext_::StatesAndFlags_::kTimeSigValid) as u32;
        self.context.state = flags;
        self.host.latency_changed.store(false, Ordering::Relaxed);
        Ok(layout)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn ara_bind(&self, track: &openpt_dsp::AraTrack) -> Option<crate::ara::Binding> {
        if self.active {
            log::info!("ARA: the instance is already active");
            return None;
        }
        unsafe { crate::ara::bind(self.component.as_ptr().cast(), track) }
    }

    pub fn set_channel_context(&self, c: &openpt_dsp::ChannelInfo) -> bool {
        use sv::ChannelContext::{self as cc, IInfoListenerTrait};
        let target = self.controller.as_ref().and_then(|x| x.cast::<cc::IInfoListener>()).or_else(|| self.component.cast::<cc::IInfoListener>());
        let Some(l) = target else { return false };
        let attrs = HostAttributes::default();
        let utf16 = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let uid = format!("openpt-track-{}", c.id);
        let name_len = c.name.encode_utf16().count();
        let [r, g, b] = c.color;
        let argb = 0xFF00_0000u32 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        attrs.set(cc::kChannelUIDKey, Attr::Str(utf16(&uid)));
        attrs.set(cc::kChannelUIDLengthKey, Attr::Int(uid.len() as i64));
        attrs.set(cc::kChannelNameKey, Attr::Str(utf16(&c.name)));
        attrs.set(cc::kChannelNameLengthKey, Attr::Int(name_len as i64));
        attrs.set(cc::kChannelIndexKey, Attr::Int(c.index as i64 + 1));
        attrs.set(cc::kChannelColorKey, Attr::Int(i64::from(argb)));
        attrs.set(cc::kChannelRuntimeIDKey, Attr::Str(utf16(&uid)));
        attrs.set(cc::kChannelPluginLocationKey, Attr::Int(i64::from(cc::ChannelPluginLocation_::kPreVolumeFader)));
        let Some(list) = ComWrapper::new(attrs).to_com_ptr::<sv::IAttributeList>() else { return false };
        ok(unsafe { l.setChannelContextInfos(list.as_ptr()) })
    }

    pub fn shutdown(&mut self) {
        if self.processing {
            if let Some(p) = self.processor() {
                unsafe { p.setProcessing(0) };
            }
            self.processing = false;
        }
        if self.active {
            unsafe { self.component.setActive(0) };
            self.active = false;
        }
    }

    pub fn process(
        &mut self,
        frames: usize,
        inputs: &mut BusBuffers,
        outputs: &mut BusBuffers,
        params: &[(u32, f64)],
        controls: &[(u32, u32, f64)],
        notes: &[NoteEvent],
        offset: u32,
        project_time: i64,
        transport: Option<&Transport>,
    ) -> Result<(), Vst3Error> {
        if !self.active {
            return Err(Vst3Error::Process("not active".into()));
        }
        if frames == 0 || frames > self.max_frames || frames > inputs.frames || frames > outputs.frames {
            return Err(Vst3Error::Process("block larger than the configured size".into()));
        }
        let p = self.processor.clone().ok_or_else(|| Vst3Error::Process("no audio processor".into()))?;
        if !self.processing {
            unsafe { p.setProcessing(1) };
            self.processing = true;
        }
        self.in_params.clear();
        for &(id, v) in params.iter().take(PARAM_QUEUE_CAP) {
            self.in_params.set(id, 0, v.clamp(0.0, 1.0));
        }
        for &(time, id, v) in controls {
            if let Some(at) = time.checked_sub(offset).filter(|&t| (t as usize) < frames) {
                self.in_params.set(id, i32::try_from(at).unwrap_or(0), v.clamp(0.0, 1.0));
            }
        }
        self.out_params.clear();
        let event_in = self.event_in;
        self.in_events.with(|v| {
            v.clear();
            if event_in {
                for n in notes.iter().take(EVENT_CAP) {
                    v.push(n.to_raw(offset));
                }
            }
        });
        self.out_events.with(Vec::clear);
        inputs.refresh();
        outputs.refresh();
        self.context.continousTimeSamples = project_time;
        match transport {
            Some(t) => apply_transport(&mut self.context, t),
            None => self.context.projectTimeSamples = project_time,
        }
        let mut data = sv::ProcessData {
            processMode: self.process_mode(),
            symbolicSampleSize: sv::SymbolicSampleSizes_::kSample32 as i32,
            numSamples: i32::try_from(frames).unwrap_or(0),
            numInputs: i32::try_from(inputs.bufs.len()).unwrap_or(0),
            numOutputs: i32::try_from(outputs.bufs.len()).unwrap_or(0),
            inputs: if inputs.bufs.is_empty() { std::ptr::null_mut() } else { inputs.bufs.as_mut_ptr() },
            outputs: if outputs.bufs.is_empty() { std::ptr::null_mut() } else { outputs.bufs.as_mut_ptr() },
            inputParameterChanges: self.in_params.as_com_ref::<sv::IParameterChanges>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputParameterChanges: self.out_params.as_com_ref::<sv::IParameterChanges>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            inputEvents: self.in_events.as_com_ref::<sv::IEventList>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputEvents: self.out_events.as_com_ref::<sv::IEventList>().map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            processContext: &mut self.context,
        };
        let r = unsafe { p.process(&mut data) };
        self.forward_outputs();
        if ok(r) { Ok(()) } else { Err(Vst3Error::Process(format!("process returned {r}"))) }
    }
}

impl Instance {
    fn forward_outputs(&self) {
        let n = usize::try_from(self.out_params.count.load(Ordering::Relaxed).max(0)).unwrap_or(0);
        if n == 0 {
            return;
        }
        let Ok(mut out) = self.editor.outputs.try_lock() else { return };
        for q in self.out_params.queues.iter().take(n) {
            if q.points.load(Ordering::Relaxed) < 1 {
                continue;
            }
            let (id, v) = (q.id.load(Ordering::Relaxed), f64::from_bits(q.value.load(Ordering::Relaxed)));
            match out.iter().position(|(i, _)| *i == id) {
                Some(k) => out[k].1 = v,
                None if out.len() < OUTPUTS_CAP => out.push((id, v)),
                None => {}
            }
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.editor.kill() {
            self.bundle.mark_leaked();
        }
        self.gain_reduction = None;
        self.shutdown();
        if let Some((a, b)) = self.connections.take() {
            unsafe {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
        }
        if let Some(c) = self.controller.take() {
            unsafe { c.setComponentHandler(std::ptr::null_mut()) };
            if self.controller_initialized {
                unsafe { c.terminate() };
            }
        }
        self.processor = None;
        if self.component_initialized {
            unsafe { self.component.terminate() };
            self.component_initialized = false;
        }
    }
}

fn split_state(data: &[u8]) -> Option<(&[u8], &[u8])> {
    if data.len() > MAX_STATE_BYTES.saturating_mul(2) {
        return None;
    }
    let Some(rest) = data.strip_prefix(STATE_MAGIC) else { return (!data.is_empty()).then_some((data, &[][..])) };
    let take = |b: &[u8]| -> Option<(usize, usize)> {
        let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
        (n <= MAX_STATE_BYTES && b.len() >= 4 + n).then_some((4, 4 + n))
    };
    let (a, b) = take(rest)?;
    let comp = rest.get(a..b)?;
    let rest = rest.get(b..)?;
    let (c, d) = take(rest)?;
    let ctrl = rest.get(c..d)?;
    (d == rest.len()).then_some((comp, ctrl))
}

pub(crate) struct PlugFrame {
    window: std::sync::atomic::AtomicPtr<c_void>,
}

impl Class for PlugFrame {
    type Interfaces = (sb::IPlugFrame,);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut sb::IPlugView, new_size: *mut sb::ViewRect) -> sb::tresult {
        let Some(r) = (unsafe { new_size.as_mut() }) else { return sb::kInvalidArgument };
        let (w, h) = rect_size(r);
        window::resize(self.window.load(Ordering::Relaxed), w, h);
        if let Some(v) = unsafe { vst3::ComRef::from_raw(view) } {
            unsafe { v.onSize(r) };
        }
        sb::kResultOk
    }
}

fn set_content_scale(view: &ComPtr<sb::IPlugView>, scale: f64) -> bool {
    let Some(s) = view.cast::<sb::IPlugViewContentScaleSupport>() else { return false };
    ok(unsafe { s.setContentScaleFactor(scale as f32) })
}

fn rect_size(r: &sb::ViewRect) -> (f64, f64) {
    let w = r.right.saturating_sub(r.left).clamp(1, 16_384);
    let h = r.bottom.saturating_sub(r.top).clamp(1, 16_384);
    (f64::from(w), f64::from(h))
}

#[derive(Default)]
struct EditorInner {
    component: Option<ComPtr<sv::IComponent>>,
    controller: Option<ComPtr<sv::IEditController>>,
    view: Option<ComPtr<sb::IPlugView>>,
    frame: Option<ComWrapper<PlugFrame>>,
    window: Option<window::HostWindow>,
    header: openpt_dsp::EditorHeader,
}

#[derive(Default)]
pub(crate) struct EditorLink {
    inner: Mutex<EditorInner>,
    outputs: Mutex<Vec<(u32, f64)>>,
}

impl EditorLink {
    fn lock(&self) -> std::sync::MutexGuard<'_, EditorInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn has_controller(&self) -> bool {
        self.lock().controller.is_some()
    }

    fn create_view(g: &EditorInner) -> Result<ComPtr<sb::IPlugView>, String> {
        let c = g.controller.as_ref().ok_or_else(|| "the plugin is gone".to_string())?;
        let raw = unsafe { c.createView(c"editor".as_ptr()) };
        unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| "this plugin has no editor".to_string())
    }

    pub fn probe(&self) -> Result<(u32, u32), String> {
        self.probe_at(None)
    }

    pub fn probe_at(&self, scale: Option<f64>) -> Result<(u32, u32), String> {
        let g = self.lock();
        let view = EditorLink::create_view(&g)?;
        if let Some(k) = scale {
            set_content_scale(&view, k);
        }
        let platform = window::PLATFORM_TYPE.ok_or_else(|| "plugin editors are not supported on this platform".to_string())?;
        if unsafe { view.isPlatformTypeSupported(platform) } != sb::kResultTrue {
            return Err("the editor does not support this platform's windows".into());
        }
        let mut r = sb::ViewRect { left: 0, top: 0, right: 0, bottom: 0 };
        if !ok(unsafe { view.getSize(&mut r) }) {
            return Err("the editor reports no size".into());
        }
        let (w, h) = rect_size(&r);
        Ok((w as u32, h as u32))
    }

    pub fn open(&self, title: &str) -> Result<(), String> {
        if !window::on_main_thread() {
            return Err("plugin editors must be opened on the main thread".into());
        }
        let platform = window::PLATFORM_TYPE.ok_or_else(|| "VST3 editors are not hosted on this platform yet".to_string())?;
        let mut g = self.lock();
        if let Some(w) = &g.window {
            if w.is_visible() {
                w.raise();
                return Ok(());
            }
            EditorLink::teardown(&mut g);
        }
        let view = EditorLink::create_view(&g)?;
        if unsafe { view.isPlatformTypeSupported(platform) } != sb::kResultTrue {
            return Err("the editor does not support this platform's windows".into());
        }
        let win = window::HostWindow::new(title, 400.0, 300.0)?;
        if let Some(k) = win.scale() {
            set_content_scale(&view, k);
        }
        let mut r = sb::ViewRect { left: 0, top: 0, right: 400, bottom: 300 };
        unsafe { view.getSize(&mut r) };
        let (w, h) = rect_size(&r);
        window::resize(win.raw(), w, h);
        let frame = ComWrapper::new(PlugFrame { window: std::sync::atomic::AtomicPtr::new(win.raw()) });
        if let Some(f) = frame.as_com_ref::<sb::IPlugFrame>() {
            unsafe { view.setFrame(f.as_ptr()) };
        }
        if !ok(unsafe { view.attached(win.parent(), platform) }) {
            unsafe { view.setFrame(std::ptr::null_mut()) };
            win.close();
            return Err("the editor could not attach to its window".into());
        }
        let resizable = unsafe { view.canResize() } == sb::kResultTrue;
        win.set_view(view.as_ptr().cast(), resizable);
        let mut r2 = r;
        if ok(unsafe { view.getSize(&mut r2) }) && rect_size(&r2) != (w, h) {
            let (w, h) = rect_size(&r2);
            window::resize(win.raw(), w, h);
        }
        win.raise();
        g.view = Some(view);
        g.frame = Some(frame);
        win.set_header(&g.header);
        g.window = Some(win);
        Ok(())
    }

    fn teardown(g: &mut EditorInner) {
        if let Some(w) = &g.window {
            w.set_view(std::ptr::null_mut(), false);
        }
        if let Some(view) = g.view.take() {
            unsafe {
                view.removed();
                view.setFrame(std::ptr::null_mut());
            }
        }
        g.frame = None;
        if let Some(w) = g.window.take() {
            w.close();
        }
    }

    pub fn sync_param(&self, id: u32, n: f64) {
        if !n.is_finite() {
            return;
        }
        let g = self.lock();
        if let Some(c) = &g.controller {
            unsafe { c.setParamNormalized(id, n.clamp(0.0, 1.0)) };
        }
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        let g = self.lock();
        let component = g.component.as_ref()?;
        let comp = Instance::read_state(|s| unsafe { component.getState(s) })?;
        let ctrl = match &g.controller {
            Some(c) => Instance::read_state(|s| unsafe { c.getState(s) }).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut out = Vec::with_capacity(12 + comp.len() + ctrl.len());
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&u32::try_from(comp.len()).ok()?.to_le_bytes());
        out.extend_from_slice(&comp);
        out.extend_from_slice(&u32::try_from(ctrl.len()).ok()?.to_le_bytes());
        out.extend_from_slice(&ctrl);
        Some(out)
    }

    pub fn close(&self) {
        if window::on_main_thread() {
            EditorLink::teardown(&mut self.lock());
        }
    }

    pub fn is_open(&self) -> bool {
        let g = self.lock();
        match &g.window {
            Some(w) if window::on_main_thread() => w.is_visible(),
            Some(_) => true,
            None => false,
        }
    }

    pub fn set_header(&self, header: &openpt_dsp::EditorHeader) {
        if !window::on_main_thread() {
            return;
        }
        let mut g = self.lock();
        if g.header != *header {
            g.header = header.clone();
        }
        if let Some(w) = &g.window {
            w.set_header(header);
        }
    }

    pub fn header_actions(&self) -> Vec<openpt_dsp::HeaderAction> {
        if !window::on_main_thread() {
            return Vec::new();
        }
        self.lock().window.as_ref().map(window::HostWindow::take_actions).unwrap_or_default()
    }

    pub fn load_state(&self, data: &[u8]) -> bool {
        let Some((comp, ctrl)) = split_state(data) else { return false };
        let g = self.lock();
        let Some(component) = &g.component else { return false };
        let stream = MemoryStream::new(comp.to_vec());
        let Some(s) = stream.as_com_ref::<sb::IBStream>() else { return false };
        if !ok(unsafe { component.setState(s.as_ptr()) }) {
            return false;
        }
        if let Some(c) = &g.controller {
            stream.lock().1 = 0;
            unsafe { c.setComponentState(s.as_ptr()) };
            if !ctrl.is_empty() {
                let cs = MemoryStream::new(ctrl.to_vec());
                if let Some(r) = cs.as_com_ref::<sb::IBStream>() {
                    unsafe { c.setState(r.as_ptr()) };
                }
            }
        }
        true
    }

    pub fn idle(&self) {
        if !window::on_main_thread() {
            return;
        }
        let mut g = self.lock();
        if g.window.as_ref().is_some_and(|w| !w.is_visible()) {
            EditorLink::teardown(&mut g);
        }
        let outputs: Vec<(u32, f64)> = self.outputs.lock().map(|mut o| o.drain(..).collect()).unwrap_or_default();
        if let Some(c) = &g.controller {
            for (id, v) in outputs {
                unsafe { c.setParamNormalized(id, v.clamp(0.0, 1.0)) };
            }
        }
    }

    pub fn value_text(&self, id: u32, normalized: f64) -> Option<String> {
        let g = self.lock();
        let c = g.controller.as_ref()?;
        let mut s: sv::String128 = [0; 128];
        ok(unsafe { c.getParamStringByValue(id, normalized.clamp(0.0, 1.0), &mut s) }).then(|| read_c16(&s)).filter(|t| !t.is_empty())
    }

    pub fn output_buses(&self) -> Vec<(String, u32)> {
        let g = self.lock();
        let Some(c) = &g.component else { return Vec::new() };
        let (audio, out) = (sv::MediaTypes_::kAudio as sv::MediaType, sv::BusDirections_::kOutput as sv::BusDirection);
        let n = unsafe { c.getBusCount(audio, out) }.clamp(0, MAX_BUSES);
        (0..n)
            .filter_map(|i| {
                let mut info: sv::BusInfo = unsafe { std::mem::zeroed() };
                ok(unsafe { c.getBusInfo(audio, out, i, &mut info) }).then(|| (read_c16(&info.name), u32::try_from(info.channelCount).unwrap_or(0)))
            })
            .collect()
    }

    pub fn normalized(&self, id: u32) -> Option<f64> {
        let g = self.lock();
        let c = g.controller.as_ref()?;
        let v = unsafe { c.getParamNormalized(id) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    pub fn to_plain(&self, id: u32, n: f64) -> Option<f64> {
        let g = self.lock();
        let c = g.controller.as_ref()?;
        let v = unsafe { c.normalizedParamToPlain(id, n.clamp(0.0, 1.0)) };
        v.is_finite().then_some(v)
    }

    pub fn to_normalized(&self, id: u32, plain: f64) -> Option<f64> {
        let g = self.lock();
        let c = g.controller.as_ref()?;
        if !plain.is_finite() {
            return None;
        }
        let v = unsafe { c.plainParamToNormalized(id, plain) };
        v.is_finite().then(|| v.clamp(0.0, 1.0))
    }

    pub fn set_normalized(&self, id: u32, n: f64) {
        if !window::on_main_thread() || !n.is_finite() {
            return;
        }
        let g = self.lock();
        if let Some(c) = &g.controller {
            unsafe { c.setParamNormalized(id, n.clamp(0.0, 1.0)) };
        }
    }

    fn kill(&self) -> bool {
        let mut g = self.lock();
        let mut leaked = false;
        if g.view.is_some() || g.window.is_some() {
            if window::on_main_thread() {
                EditorLink::teardown(&mut g);
            } else {
                log::warn!("vst3: editor still open while its plugin is destroyed off the main thread");
                if let Some(v) = g.view.take() {
                    std::mem::forget(v);
                    leaked = true;
                }
                g.window = None;
                if let Some(f) = g.frame.take() {
                    std::mem::forget(f);
                }
            }
        }
        g.component = None;
        g.controller = None;
        leaked
    }
}

pub(crate) fn pump_editor_timers() {
    window::pump_timers();
}


#[cfg(target_os = "macos")]
mod window {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSBackingStoreType, NSFloatingWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::ffi::{c_char, c_void};

    pub const PLATFORM_TYPE: Option<*const c_char> = Some(vst3::Steinberg::kPlatformTypeNSView);

    pub fn pump_timers() {}

    pub fn on_main_thread() -> bool {
        MainThreadMarker::new().is_some()
    }

    pub struct HostWindow {
        window: *mut NSWindow,
    }

    unsafe impl Send for HostWindow {}

    impl HostWindow {
        fn get(&self) -> Option<&NSWindow> {
            MainThreadMarker::new()?;
            unsafe { self.window.as_ref() }
        }

        pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
            let mtm = MainThreadMarker::new().ok_or_else(|| "not on the main thread".to_string())?;
            let _app = NSApplication::sharedApplication(mtm);
            let rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(w, h));
            let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false)
            };
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(&NSString::from_str(title));
            window.setLevel(NSFloatingWindowLevel);
            window.setHidesOnDeactivate(true);
            window.setExcludedFromWindowsMenu(true);
            window.setCollectionBehavior(NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle);
            window.center();
            Ok(HostWindow { window: Retained::into_raw(window) })
        }

        pub fn scale(&self) -> Option<f64> {
            None
        }

        pub fn set_view(&self, _view: *mut c_void, _resizable: bool) {}

        pub fn raw(&self) -> *mut c_void {
            self.window.cast()
        }

        pub fn parent(&self) -> *mut c_void {
            self.get().and_then(|w| w.contentView()).map_or(std::ptr::null_mut(), |v| Retained::as_ptr(&v).cast_mut().cast())
        }

        pub fn raise(&self) {
            if let Some(w) = self.get() {
                w.makeKeyAndOrderFront(None);
            }
        }

        pub fn set_header(&self, _header: &openpt_dsp::EditorHeader) {}

        pub fn take_actions(&self) -> Vec<openpt_dsp::HeaderAction> {
            Vec::new()
        }

        pub fn is_visible(&self) -> bool {
            self.get().is_some_and(|w| w.isVisible())
        }

        pub fn close(self) {
            if let Some(w) = self.get() {
                w.orderOut(None);
                w.close();
            }
            if MainThreadMarker::new().is_some() {
                drop(unsafe { Retained::from_raw(self.window) });
            }
        }
    }

    pub fn resize(window: *mut c_void, w: f64, h: f64) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        if let Some(win) = unsafe { window.cast::<NSWindow>().as_ref() } {
            win.setContentSize(NSSize::new(w, h));
        }
    }
}

#[cfg(windows)]
mod window {
    use openpt_dsp::{EditorHeader, HeaderAction};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;
    use vst3::Steinberg::{self as sb, IPlugViewTrait};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
        DeleteObject, DrawTextW, EndPaint, FillRect, GetMonitorInfoW, HDC, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GA_ROOT, GW_OWNER, GetAncestor, GetForegroundWindow, GetWindow, GetWindowRect, SWP_NOSIZE,
        AdjustWindowRectEx, AppendMenuW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DestroyWindow, GWL_EXSTYLE, GWL_STYLE, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, IDC_ARROW, IsWindow, IsWindowVisible, LoadCursorW,
        MF_CHECKED, MF_SEPARATOR, MF_STRING, MoveWindow, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_TOPALIGN, TrackPopupMenu,
        WM_CLOSE, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_PAINT, WM_SIZE, WM_SIZING, WMSZ_BOTTOMLEFT, WMSZ_LEFT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT,
        WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_MAXIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
    };

    pub const PLATFORM_TYPE: Option<*const c_char> = Some(vst3::Steinberg::kPlatformTypeHWND);

    pub fn on_main_thread() -> bool {
        std::thread::current().name() == Some("main")
    }

    fn app_window() -> HWND {
        unsafe extern "system" fn pick(h: HWND, out: LPARAM) -> i32 {
            unsafe {
                if IsWindowVisible(h) != 0 && GetWindow(h, GW_OWNER).is_null() && with_state(h, |_| ()).is_none() {
                    *(out as *mut HWND) = h;
                    return 0;
                }
            }
            1
        }
        let mut found: HWND = std::ptr::null_mut();
        unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(pick), &mut found as *mut HWND as LPARAM) };
        found
    }

    fn give_back_focus(h: HWND) {
        unsafe {
            let mut w = GetAncestor(GetForegroundWindow(), GA_ROOT);
            while !w.is_null() && w != h {
                w = GetWindow(w, GW_OWNER);
            }
            if w.is_null() {
                return;
            }
            let app = app_window();
            if !app.is_null() {
                SetForegroundWindow(app);
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().filter(|&c| c != 0).chain(Some(0)).collect()
    }

    const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    const HEADER: f64 = 46.0;
    const HEADER2: f64 = 86.0;

    struct State {
        header: EditorHeader,
        actions: Vec<HeaderAction>,
        content: HWND,
        resizable: bool,
        view_size: (i32, i32),
    }

    thread_local! {
        static PLUGIN_RESIZING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static STATES: RefCell<HashMap<usize, State>> = RefCell::new(HashMap::new());
    }

    pub fn pump_timers() {
        use windows_sys::Win32::Graphics::Gdi::{RDW_ALLCHILDREN, RDW_UPDATENOW, RedrawWindow};
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, EnumChildWindows, EnumThreadWindows, GetClassNameW, MSG, PM_REMOVE, PeekMessageW, WM_TIMER,
        };
        if !on_main_thread() {
            return;
        }
        unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> i32 {
            unsafe { (*(l as *mut Vec<HWND>)).push(h) };
            1
        }
        let hosts: Vec<HWND> = STATES.with(|s| s.borrow().keys().map(|k| *k as HWND).collect());
        if hosts.is_empty() {
            return;
        }
        let winit = |h: HWND| {
            let mut buf = [0u16; 32];
            let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
            String::from_utf16_lossy(&buf[..n]) == "Window Class"
        };
        let mut tops: Vec<HWND> = Vec::new();
        unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(collect), &mut tops as *mut Vec<HWND> as LPARAM) };
        let mut targets: Vec<HWND> = vec![-1isize as HWND];
        for h in tops.into_iter().filter(|h| !winit(*h)) {
            targets.push(h);
            unsafe { EnumChildWindows(h, Some(collect), &mut targets as *mut Vec<HWND> as LPARAM) };
        }
        for h in targets {
            for _ in 0..8 {
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                if unsafe { PeekMessageW(&mut msg, h, WM_TIMER, WM_TIMER, PM_REMOVE) } == 0 {
                    break;
                }
                unsafe { DispatchMessageW(&msg) };
            }
        }
        for h in hosts {
            unsafe { RedrawWindow(h, std::ptr::null(), std::ptr::null_mut(), RDW_UPDATENOW | RDW_ALLCHILDREN) };
        }
    }

    fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        STATES.with(|s| s.borrow_mut().get_mut(&(hwnd as usize)).map(f))
    }

    fn view_of(hwnd: HWND) -> Option<vst3::ComRef<'static, sb::IPlugView>> {
        unsafe { vst3::ComRef::from_raw(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut sb::IPlugView) }
    }

    fn dpi_scale(hwnd: HWND) -> f64 {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 { f64::from(dpi) / 96.0 } else { 1.0 }
    }

    fn header_px(hwnd: HWND) -> i32 {
        let mut c = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(hwnd, &mut c) };
        header_px_at(hwnd, c.right)
    }

    fn header_px_at(hwnd: HWND, width: i32) -> i32 {
        let k = dpi_scale(hwnd);
        with_state(hwnd, |s| header_height(&s.header, width, k)).unwrap_or_else(|| (HEADER * k).round() as i32)
    }

    fn frame_insets(hwnd: HWND) -> RECT {
        let (style, ex) = unsafe { (GetWindowLongPtrW(hwnd, GWL_STYLE) as u32, GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32) };
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { AdjustWindowRectEx(&mut r, style, 0, ex) };
        r
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Item {
        Track,
        Preset,
        Prev,
        Next,
        Key,
        ExtKey,
        Listen,
        Auto,
        Bypass,
    }

    fn one_row_width(h: &EditorHeader, k: f64) -> i32 {
        ((if h.takes_key { 794.0 } else { 512.0 }) * k).round() as i32
    }

    fn header_height(h: &EditorHeader, width: i32, k: f64) -> i32 {
        ((if width >= one_row_width(h, k) { HEADER } else { HEADER2 }) * k).round() as i32
    }

    fn layout(h: &EditorHeader, width: i32, k: f64) -> Vec<(Item, RECT, &'static str)> {
        let px = |v: f64| (v * k).round() as i32;
        let one_row = width >= one_row_width(h, k);
        let row = |y: f64| (px(y), px(y + 21.0));
        let (r1, r2) = (row(19.0), if one_row { row(19.0) } else { row(59.0) });
        let at = |(top, bottom): (i32, i32), x0: f64, x1: f64| RECT { left: px(x0), top, right: px(x1), bottom };
        let mut out = vec![
            (Item::Track, at(r1, 8.0, 140.0), "Track"),
            (Item::Preset, at(r1, 148.0, 318.0), "Preset"),
            (Item::Prev, at(r1, 322.0, 342.0), ""),
            (Item::Next, at(r1, 344.0, 364.0), ""),
        ];
        if h.takes_key {
            let x = if one_row { 378.0 } else { 8.0 };
            out.push((Item::Key, at(r2, x, x + 112.0), "Key Input"));
            out.push((Item::ExtKey, at(r2, x + 116.0, x + 194.0), ""));
            out.push((Item::Listen, at(r2, x + 198.0, x + 268.0), ""));
        }
        let right = width - px(8.0);
        out.push((Item::Auto, RECT { left: right - px(122.0), top: r2.0, right: right - px(70.0), bottom: r2.1 }, ""));
        out.push((Item::Bypass, RECT { left: right - px(64.0), top: r2.0, right, bottom: r2.1 }, ""));
        out
    }

    fn min_width(hwnd: HWND) -> i32 {
        (448.0 * dpi_scale(hwnd)).round() as i32
    }

    fn rgb(r: u8, g: u8, b: u8) -> u32 {
        u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
    }

    fn fill(hdc: HDC, r: &RECT, color: u32) {
        unsafe {
            let b = CreateSolidBrush(color);
            FillRect(hdc, r, b);
            DeleteObject(b);
        }
    }

    fn text(hdc: HDC, s: &str, r: &RECT, color: u32, flags: u32) {
        if s.is_empty() {
            return;
        }
        let w = wide(s);
        let mut r = *r;
        unsafe {
            SetTextColor(hdc, color);
            DrawTextW(hdc, w.as_ptr(), -1, &mut r, flags | DT_SINGLELINE | DT_VCENTER);
        }
    }

    fn paint(hwnd: HWND) {
        let Some(h) = with_state(hwnd, |s| s.header.clone()) else { return };
        let k = dpi_scale(hwnd);
        let px = |v: f64| (v * k).round() as i32;
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
        if hdc.is_null() {
            return;
        }
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(hwnd, &mut client) };
        fill(hdc, &client, rgb(36, 38, 41));
        let strip = RECT { left: 0, top: 0, right: client.right, bottom: header_px(hwnd) };
        fill(hdc, &strip, rgb(52, 54, 58));
        fill(hdc, &RECT { top: strip.bottom - 1, ..strip }, rgb(20, 21, 23));
        let face = wide("Segoe UI");
        let (font, small) = unsafe {
            (
                CreateFontW(-px(12.0), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()),
                CreateFontW(-px(10.0), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()),
            )
        };
        let old = unsafe {
            SetBkMode(hdc, TRANSPARENT as i32);
            SelectObject(hdc, small)
        };
        let dim = rgb(160, 165, 170);
        let bright = rgb(228, 230, 232);
        let lit_key = rgb(232, 160, 40);
        let items = layout(&h, client.right, k);
        for (_, r, label) in &items {
            if !label.is_empty() {
                text(hdc, label, &RECT { left: r.left, top: r.top - px(16.0), right: r.right, bottom: r.top - px(2.0) }, dim, DT_LEFT);
            }
        }
        unsafe { SelectObject(hdc, font) };
        for (item, r, _) in &items {
            let inner = RECT { left: r.left + px(6.0), right: r.right - px(6.0), ..*r };
            match item {
                Item::Track => {
                    fill(hdc, r, rgb(14, 16, 16));
                    text(hdc, &h.track, &inner, bright, DT_LEFT | DT_END_ELLIPSIS);
                }
                Item::Preset => {
                    fill(hdc, r, rgb(14, 16, 16));
                    let name = if h.preset.is_empty() { "<factory default>" } else { h.preset.as_str() };
                    text(hdc, name, &RECT { right: inner.right - px(12.0), ..inner }, bright, DT_LEFT | DT_END_ELLIPSIS);
                    text(hdc, "▾", &inner, dim, DT_RIGHT);
                }
                Item::Prev | Item::Next => {
                    fill(hdc, r, rgb(70, 72, 76));
                    text(hdc, if *item == Item::Prev { "◀" } else { "▶" }, r, bright, DT_CENTER);
                }
                Item::Key => {
                    fill(hdc, r, rgb(14, 16, 16));
                    let name = h.key.and_then(|i| h.busses.get(i)).map_or("no key input", String::as_str);
                    text(
                        hdc,
                        name,
                        &RECT { right: inner.right - px(12.0), ..inner },
                        if h.key.is_some() { bright } else { dim },
                        DT_LEFT | DT_END_ELLIPSIS,
                    );
                    text(hdc, "▾", &inner, dim, DT_RIGHT);
                }
                Item::ExtKey | Item::Listen => {
                    let on = if *item == Item::ExtKey { h.key_on } else { h.key_listen };
                    let enabled = if *item == Item::ExtKey { h.key.is_some() } else { h.key_on };
                    fill(hdc, r, if on { lit_key } else { rgb(70, 72, 76) });
                    let label = if *item == Item::ExtKey { "External Key" } else { "Key Listen" };
                    text(
                        hdc,
                        label,
                        r,
                        if on {
                            rgb(20, 20, 20)
                        } else if enabled {
                            bright
                        } else {
                            rgb(110, 112, 116)
                        },
                        DT_CENTER,
                    );
                }
                Item::Auto => {
                    fill(hdc, r, rgb(70, 72, 76));
                    text(hdc, "Auto", r, if h.params.is_empty() { rgb(110, 112, 116) } else { bright }, DT_CENTER);
                }
                Item::Bypass => {
                    fill(hdc, r, if h.bypass { rgb(40, 110, 200) } else { rgb(70, 72, 76) });
                    text(hdc, "Bypass", r, if h.bypass { rgb(255, 255, 255) } else { bright }, DT_CENTER);
                }
            }
        }
        unsafe {
            SelectObject(hdc, old);
            DeleteObject(font);
            DeleteObject(small);
            EndPaint(hwnd, &ps);
        }
    }

    fn popup(hwnd: HWND, r: &RECT, entries: &[(usize, String, bool)]) -> usize {
        unsafe {
            let m = CreatePopupMenu();
            if m.is_null() {
                return 0;
            }
            for (id, label, checked) in entries {
                if *id == 0 {
                    AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
                } else {
                    let w = wide(label);
                    AppendMenuW(m, MF_STRING | if *checked { MF_CHECKED } else { 0 }, *id, w.as_ptr());
                }
            }
            let mut p = POINT { x: r.left, y: r.bottom };
            ClientToScreen(hwnd, &mut p);
            let picked = TrackPopupMenu(m, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, p.x, p.y, 0, hwnd, std::ptr::null());
            DestroyMenu(m);
            usize::try_from(picked).unwrap_or(0)
        }
    }

    fn click(hwnd: HWND, x: i32, y: i32) {
        let Some(h) = with_state(hwnd, |s| s.header.clone()) else { return };
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(hwnd, &mut client) };
        let hit = layout(&h, client.right, dpi_scale(hwnd)).into_iter().find(|(_, r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom);
        let Some((item, r, _)) = hit else { return };
        let action = match item {
            Item::Track => None,
            Item::Preset => {
                let mut e = vec![(1, "<factory default>".to_string(), h.preset == "<factory default>")];
                if !h.presets.is_empty() {
                    e.push((0, String::new(), false));
                }
                e.extend(h.presets.iter().enumerate().map(|(i, n)| (i + 10, n.clone(), *n == h.preset)));
                e.push((0, String::new(), false));
                e.push((2, "Save Preset".to_string(), false));
                match popup(hwnd, &r, &e) {
                    0 => None,
                    1 => Some(HeaderAction::Preset(None)),
                    2 => Some(HeaderAction::SavePreset),
                    n => Some(HeaderAction::Preset(Some(n - 10))),
                }
            }
            Item::Prev => Some(HeaderAction::PrevPreset),
            Item::Next => Some(HeaderAction::NextPreset),
            Item::Key => {
                let mut e = vec![(1, "no key input".to_string(), h.key.is_none()), (0, String::new(), false)];
                e.extend(h.busses.iter().enumerate().map(|(i, n)| (i + 10, n.clone(), h.key == Some(i))));
                match popup(hwnd, &r, &e) {
                    0 => None,
                    1 => Some(HeaderAction::Key(None)),
                    n => Some(HeaderAction::Key(Some(n - 10))),
                }
            }
            Item::ExtKey => h.key.is_some().then_some(HeaderAction::ExternalKey),
            Item::Listen => h.key_on.then_some(HeaderAction::KeyListen),
            Item::Auto => {
                let e: Vec<(usize, String, bool)> = h.params.iter().enumerate().map(|(i, n)| (i + 10, n.clone(), false)).collect();
                match popup(hwnd, &r, &e) {
                    0 => None,
                    n => Some(HeaderAction::Automate(n - 10)),
                }
            }
            Item::Bypass => Some(HeaderAction::Bypass),
        };
        if let Some(a) = action {
            with_state(hwnd, |s| s.actions.push(a));
            crate::wake_ui();
        }
    }

    fn place_content(hwnd: HWND, w: i32, h: i32) {
        let top = header_px_at(hwnd, w);
        let ch = (h - top).max(1);
        let Some((c, (vw, _))) = with_state(hwnd, |s| (s.content, s.view_size)) else { return };
        let cw = if vw > 0 { vw.min(w) } else { w };
        unsafe { MoveWindow(c, ((w - cw) / 2).max(0), top, cw, ch, 1) };
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let ours = with_state(hwnd, |_| ()).is_some();
        if !ours && msg == WM_ERASEBKGND {
            let mut c = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe { GetClientRect(hwnd, &mut c) };
            fill(wparam as HDC, &c, rgb(36, 38, 41));
            return 1;
        }
        if ours && msg == WM_CLOSE {
            give_back_focus(hwnd);
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
        if ours && msg == WM_PAINT {
            paint(hwnd);
            return 0;
        }
        if ours && msg == WM_LBUTTONDOWN {
            click(hwnd, i32::from((lparam & 0xFFFF) as u16 as i16), i32::from(((lparam >> 16) & 0xFFFF) as u16 as i16));
            return 0;
        }
        if ours
            && msg == WM_SIZING
            && let Some(v) = view_of(hwnd)
        {
            let Some(w) = (unsafe { (lparam as *mut RECT).as_mut() }) else { return 1 };
            let f = frame_insets(hwnd);
            let top = header_px_at(hwnd, (w.right - f.right) - (w.left - f.left));
            let mut r = sb::ViewRect {
                left: 0,
                top: 0,
                right: (w.right - f.right) - (w.left - f.left),
                bottom: ((w.bottom - f.bottom) - (w.top - f.top) - top).max(1),
            };
            if unsafe { v.checkSizeConstraint(&mut r) } == sb::kResultTrue {
                let cw = (r.right - r.left).max(min_width(hwnd));
                let ch = r.bottom - r.top + header_px_at(hwnd, cw);
                let (ow, oh) = (cw + f.right - f.left, ch + f.bottom - f.top);
                if matches!(wparam as u32, WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT) {
                    w.left = w.right - ow;
                } else {
                    w.right = w.left + ow;
                }
                if matches!(wparam as u32, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT) {
                    w.top = w.bottom - oh;
                } else {
                    w.bottom = w.top + oh;
                }
            }
            return 1;
        }
        if ours && msg == WM_SIZE {
            let (w, h) = ((lparam & 0xFFFF) as i32, ((lparam >> 16) & 0xFFFF) as i32);
            let avail = (w, (h - header_px_at(hwnd, w)).max(1));
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
            if !PLUGIN_RESIZING.with(std::cell::Cell::get)
                && with_state(hwnd, |s| s.resizable) == Some(true)
                && let Some(v) = view_of(hwnd)
            {
                let mut r = sb::ViewRect { left: 0, top: 0, right: avail.0, bottom: avail.1 };
                let fits = r.right > 0 && r.bottom > 0 && unsafe { v.checkSizeConstraint(&mut r) } == sb::kResultTrue;
                let size = (r.right - r.left, r.bottom - r.top);
                if fits && size.0 > 0 && size.1 > 0 && with_state(hwnd, |s| std::mem::replace(&mut s.view_size, size)) != Some(size) {
                    unsafe { v.onSize(&mut r) };
                }
            }
            place_content(hwnd, w, h);
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn class_name() -> Option<&'static [u16]> {
        static CLASS: OnceLock<Option<Vec<u16>>> = OnceLock::new();
        CLASS
            .get_or_init(|| {
                let name = wide("OpenPTVst3Editor");
                let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: instance,
                    hIcon: std::ptr::null_mut(),
                    hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: name.as_ptr(),
                    hIconSm: std::ptr::null_mut(),
                };
                (unsafe { RegisterClassExW(&wc) } != 0).then_some(name)
            })
            .as_deref()
    }

    fn outer_size(style: u32, ex_style: u32, w: f64, h: f64) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
        unsafe { AdjustWindowRectEx(&mut r, style, 0, ex_style) };
        (r.right.saturating_sub(r.left), r.bottom.saturating_sub(r.top))
    }

    pub struct HostWindow {
        hwnd: HWND,
    }

    unsafe impl Send for HostWindow {}

    impl HostWindow {
        fn live(&self) -> Option<HWND> {
            (on_main_thread() && unsafe { IsWindow(self.hwnd) } != 0).then_some(self.hwnd)
        }

        pub fn new(title: &str, w: f64, h: f64) -> Result<HostWindow, String> {
            if !on_main_thread() {
                return Err("not on the main thread".into());
            }
            let class = class_name().ok_or_else(|| "cannot register the editor window class".to_string())?;
            let title = wide(title);
            let (ow, oh) = outer_size(STYLE, 0, w, h + HEADER);
            let owner = match app_window() {
                h if h.is_null() => {
                    unsafe { GetActiveWindow() }
                }
                h => h,
            };
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    STYLE,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    ow,
                    oh,
                    owner,
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err("cannot create the editor window".into());
            }
            let top = header_px(hwnd);
            let content = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                    0,
                    top,
                    w as i32,
                    h as i32,
                    hwnd,
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                )
            };
            if content.is_null() {
                unsafe { DestroyWindow(hwnd) };
                return Err("cannot create the editor window".into());
            }
            STATES.with(|s| {
                s.borrow_mut().insert(
                    hwnd as usize,
                    State { header: EditorHeader::default(), actions: Vec::new(), content, resizable: false, view_size: (0, 0) },
                )
            });
            Ok(HostWindow { hwnd })
        }

        pub fn scale(&self) -> Option<f64> {
            let h = self.live()?;
            let dpi = unsafe { GetDpiForWindow(h) };
            (dpi > 0).then(|| f64::from(dpi) / 96.0)
        }

        pub fn set_view(&self, view: *mut c_void, resizable: bool) {
            let Some(h) = self.live() else { return };
            unsafe {
                SetWindowLongPtrW(h, GWLP_USERDATA, view as isize);
                with_state(h, |s| s.resizable = resizable && !view.is_null());
                let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
                let style =
                    if resizable && !view.is_null() { style | WS_THICKFRAME | WS_MAXIMIZEBOX } else { style & !(WS_THICKFRAME | WS_MAXIMIZEBOX) };
                let mut c = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(h, &mut c);
                SetWindowLongPtrW(h, GWL_STYLE, style as isize);
                let (ow, oh) = outer_size(style, GetWindowLongPtrW(h, GWL_EXSTYLE) as u32, f64::from(c.right), f64::from(c.bottom));
                PLUGIN_RESIZING.with(|f| f.set(true));
                SetWindowPos(h, std::ptr::null_mut(), 0, 0, ow, oh, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                PLUGIN_RESIZING.with(|f| f.set(false));
            }
        }

        pub fn set_header(&self, header: &EditorHeader) {
            let Some(h) = self.live() else { return };
            let changed = with_state(h, |s| {
                let c = s.header != *header;
                if c {
                    s.header = header.clone();
                }
                c
            });
            if changed == Some(true) {
                let mut c = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                unsafe { GetClientRect(h, &mut c) };
                let (vw, vh) = with_state(h, |s| s.view_size).unwrap_or((0, 0));
                if vw > 0 && vh > 0 {
                    resize(h, f64::from(vw), f64::from(vh));
                } else if c.right < min_width(h) {
                    resize(h, f64::from(c.right), f64::from(c.bottom - header_px(h)));
                }
                unsafe { InvalidateRect(h, std::ptr::null(), 0) };
            }
        }

        pub fn take_actions(&self) -> Vec<HeaderAction> {
            self.live().and_then(|h| with_state(h, |s| std::mem::take(&mut s.actions))).unwrap_or_default()
        }

        pub fn raw(&self) -> *mut c_void {
            self.hwnd
        }

        pub fn parent(&self) -> *mut c_void {
            self.live().and_then(|h| with_state(h, |s| s.content)).unwrap_or(std::ptr::null_mut())
        }

        pub fn raise(&self) {
            if let Some(h) = self.live() {
                unsafe {
                    ShowWindow(h, SW_SHOW);
                    SetForegroundWindow(h);
                }
                keep_on_screen(h);
            }
        }

        pub fn is_visible(&self) -> bool {
            self.live().is_some_and(|h| unsafe { IsWindowVisible(h) } != 0)
        }

        pub fn close(self) {
            if let Some(h) = self.live() {
                give_back_focus(h);
                STATES.with(|s| s.borrow_mut().remove(&(h as usize)));
                unsafe { DestroyWindow(h) };
            }
        }
    }

    pub fn resize(window: *mut c_void, w: f64, h: f64) {
        if !on_main_thread() || window.is_null() || unsafe { IsWindow(window) } == 0 {
            return;
        }
        with_state(window, |s| s.view_size = (w.round() as i32, h.round() as i32));
        let (style, ex) = unsafe { (GetWindowLongPtrW(window, GWL_STYLE) as u32, GetWindowLongPtrW(window, GWL_EXSTYLE) as u32) };
        let w = w.max(f64::from(min_width(window)));
        let top = f64::from(header_px_at(window, w.round() as i32));
        let (ow, oh) = outer_size(style, ex, w, h + top);
        PLUGIN_RESIZING.with(|f| f.set(true));
        unsafe { SetWindowPos(window, std::ptr::null_mut(), 0, 0, ow, oh, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) };
        PLUGIN_RESIZING.with(|f| f.set(false));
        let mut c = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(window, &mut c) };
        place_content(window, c.right, c.bottom);
        keep_on_screen(window);
    }

    fn keep_on_screen(h: HWND) {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, rcMonitor: r, rcWork: r, dwFlags: 0 };
        unsafe {
            if GetWindowRect(h, &mut r) == 0 || GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
                return;
            }
        }
        let w = mi.rcWork;
        let fit = |lo: i32, size: i32, min: i32, max: i32| if size >= max - min { min } else { lo.clamp(min, max - size) };
        let (x, y) = (fit(r.left, r.right - r.left, w.left, w.right), fit(r.top, r.bottom - r.top, w.top, w.bottom));
        if (x, y) != (r.left, r.top) {
            unsafe { SetWindowPos(h, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod window {
    use std::ffi::{c_char, c_void};

    pub const PLATFORM_TYPE: Option<*const c_char> = None;

    pub fn pump_timers() {}

    pub fn on_main_thread() -> bool {
        true
    }

    pub struct HostWindow;

    impl HostWindow {
        pub fn new(_title: &str, _w: f64, _h: f64) -> Result<HostWindow, String> {
            Err("VST3 editors are not hosted on this platform yet".into())
        }
        pub fn scale(&self) -> Option<f64> {
            None
        }

        pub fn set_view(&self, _view: *mut c_void, _resizable: bool) {}

        pub fn raw(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
        pub fn parent(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
        pub fn raise(&self) {}
        pub fn set_header(&self, _header: &openpt_dsp::EditorHeader) {}

        pub fn take_actions(&self) -> Vec<openpt_dsp::HeaderAction> {
            Vec::new()
        }

        pub fn is_visible(&self) -> bool {
            false
        }
        pub fn close(self) {}
    }

    pub fn resize(_window: *mut c_void, _w: f64, _h: f64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transport_fills_the_process_context() {
        use sv::ProcessContext_::StatesAndFlags_::*;
        let mut c: sv::ProcessContext = unsafe { std::mem::zeroed() };
        let t = Transport {
            playing: true,
            tempo: 90.0,
            numerator: 3,
            denominator: 4,
            samples: 96_000,
            quarters: 3.5,
            bar_start: 3.0,
            ..Transport::default()
        };
        apply_transport(&mut c, &t);
        let playing = kPlaying as u32;
        assert_eq!(c.state & playing, playing);
        assert_eq!(c.state & kRecording as u32, 0);
        assert_eq!((c.tempo, c.timeSigNumerator, c.timeSigDenominator), (90.0, 3, 4));
        assert_eq!((c.projectTimeSamples, c.projectTimeMusic, c.barPositionMusic), (96_000, 3.5, 3.0));
        apply_transport(&mut c, &Transport { playing: false, ..t });
        assert_eq!(c.state & playing, 0);
    }

    #[test]
    fn state_container_parses_strictly() {
        let mut blob = b"SCV3".to_vec();
        blob.extend_from_slice(&3u32.to_le_bytes());
        blob.extend_from_slice(b"abc");
        blob.extend_from_slice(&2u32.to_le_bytes());
        blob.extend_from_slice(b"xy");
        assert_eq!(split_state(&blob), Some((&b"abc"[..], &b"xy"[..])));
        assert_eq!(split_state(b"raw component"), Some((&b"raw component"[..], &[][..])), "bare component state");
        assert_eq!(split_state(b""), None);
        let mut trailing = blob.clone();
        trailing.push(0);
        assert_eq!(split_state(&trailing), None);
        assert_eq!(split_state(&blob[..blob.len() - 1]), None);
        let mut huge = b"SCV3".to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(split_state(&huge), None);
        assert_eq!(split_state(b"SCV3"), None);
    }

    #[test]
    fn strings_are_bounded() {
        let mut b = [0u16; 4];
        write_c16("OpenPT", &mut b);
        assert_eq!(read_c16(&b), "Ope");
        let full: Vec<u16> = "abcd".encode_utf16().collect();
        assert_eq!(read_c16(&full), "abcd");
        let c: Vec<c_char> = b"Fx|EQ\0junk".iter().map(|&x| x as c_char).collect();
        assert_eq!(read_c8(&c), "Fx|EQ");
        write_c16("x", &mut []);
    }

    #[test]
    fn host_objects_behave() {
        let pc = ParamChanges::new(2);
        pc.set(5, 0, 0.25);
        pc.set(5, 0, 0.5);
        pc.set(6, 0, 1.0);
        pc.set(7, 0, 1.0);
        unsafe {
            assert_eq!(pc.getParameterCount(), 2, "full pool drops extra params");
            assert!(pc.getParameterData(2).is_null());
            assert!(pc.getParameterData(-1).is_null());
            let q = pc.queues[0].as_com_ref::<sv::IParamValueQueue>().unwrap();
            let (mut o, mut v) = (9, 0.0);
            assert_eq!(q.getPoint(0, &mut o, &mut v), sb::kResultOk);
            assert_eq!((o, v), (0, 0.5));
            assert_ne!(q.getPoint(1, &mut o, &mut v), sb::kResultOk);
            assert_ne!(q.getPoint(0, std::ptr::null_mut(), &mut v), sb::kResultOk);
        }
        let el = EventList::new(1);
        let mut e = NoteEvent::On { time: 5, channel: 0, key: 60, velocity: 1.0 }.to_raw(2);
        unsafe {
            assert_eq!(el.addEvent(&mut e), sb::kResultOk);
            assert_ne!(el.addEvent(&mut e), sb::kResultOk, "bounded");
            assert_eq!(el.getEventCount(), 1);
            let mut out: sv::Event = std::mem::zeroed();
            assert_eq!(el.getEvent(0, &mut out), sb::kResultOk);
            assert_eq!(out.sampleOffset, 3);
            assert_eq!(out.__field0.noteOn.pitch, 60);
            assert_ne!(el.getEvent(1, &mut out), sb::kResultOk);
        }
        let s = MemoryStream { inner: Mutex::new((Vec::new(), 0)) };
        let mut buf = *b"hello";
        unsafe {
            let mut n = 0;
            assert_eq!(s.write(buf.as_mut_ptr().cast(), 5, &mut n), sb::kResultOk);
            assert_eq!(n, 5);
            let mut pos = 0i64;
            assert_eq!(s.seek(1, sb::IBStream_::IStreamSeekMode_::kIBSeekSet as i32, &mut pos), sb::kResultOk);
            let mut out = [0u8; 8];
            assert_eq!(s.read(out.as_mut_ptr().cast(), 8, &mut n), sb::kResultOk);
            assert_eq!((n, &out[..4]), (4, &b"ello"[..]));
            assert_eq!(s.seek(-100, sb::IBStream_::IStreamSeekMode_::kIBSeekEnd as i32, &mut pos), sb::kResultOk);
            assert_eq!(pos, 0);
            assert_eq!(s.write(std::ptr::null_mut(), 5, &mut n), sb::kResultFalse);
        }
        let h = HostContext::default();
        let mut name: sv::String128 = [0; 128];
        unsafe {
            assert_eq!(h.getName(&mut name), sb::kResultOk);
            assert_eq!(h.restartComponent(sv::RestartFlags_::kLatencyChanged as i32), sb::kResultOk);
        }
        assert_eq!(read_c16(&name), "OpenPT");
        assert!(h.latency_changed.load(Ordering::Relaxed));
        assert_eq!(hex(&[0xAB; 16]), "AB".repeat(16));
    }
}
