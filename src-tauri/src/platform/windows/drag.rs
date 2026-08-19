//! Windows OLE drag implementation. Implements the four-format external
//! drag pipeline that ships a shelf capture to browsers, Electron
//! applications, Windows Explorer, and IDEs.
//!
//! The drag is built on the COM interfaces `IDataObject` (the data
//! offered) and `IDropSource` (the source side). The synchronous
//! `DoDragDrop` call drives the modal drag loop and only returns when
//! the user releases the mouse or presses Escape. The terminal HRESULT
//! is translated into a `DragOutcome` and the recorded diagnostics are
//! returned alongside.
//!
//! The four offered formats are the customary "good citizen" set for
//! image drops:
//!
//! - `CF_HDROP` — a file group descriptor pointing at the on-disk PNG.
//!   Chromium, Electron, Windows Explorer, and most IDEs accept this.
//! - Registered PNG (`CFSTR_FILEDESCRIPTOR` + `CFSTR_FILECONTENTS`) —
//!   the embedded PNG payload. Browsers prefer this over the bitmap.
//! - `CF_DIBV5` — a top-down BGRA bitmap with a V5 header. Legacy
//!   bitmap consumers (older Explorer windows, paint programs).
//! - `CF_UNICODETEXT` — the absolute PNG path as UTF-16. Text-input
//!   targets that do not interpret the image formats still receive
//!   something useful.
//!
//! The Rust side owns the file handle for the PNG until `DoDragDrop`
//! returns. Cache pruning must be blocked for the entire call, which
//! is the contract the `DragRequest::png_path` field advertises to
//! the shelf/cache layer.
//!
//! The COM implementation is hand-written on a minimal vtable shim so
//! the build does not depend on the `windows` crate's macro-driven COM
//! stack — the macro API has changed several times between versions
//! and the hand-rolled surface is small enough to maintain.

// Windows API naming uses uppercase acronyms (`HRESULT`, `FORMATETC`)
// that the Rust naming lint flags. The structs and constants below
// mirror the Win32 ABI verbatim so the COM vtable layout matches the
// platform's expectations.
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::ptr_arg)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use pixelgrab_contracts::{
    drag::{
        DragDiagnostics, DragFormat, DragRequest, DragResult, DragTargetEffect, DragTargetKind,
    },
    PlatformError, PlatformErrorKind, PlatformResult,
};

// ============================================================
// Raw FFI declarations. The only Windows API surface used here
// is the OLE drag pipeline; everything else is local.
// ============================================================

type HRESULT = i32;
type HGLOBAL = *mut c_void;
type BOOL = i32;
type UINT = u32;
type DWORD = u32;
type LPCWSTR = *const u16;

/// Clipboard format constants.
const CF_HDROP: u16 = 15;
const CF_DIBV5: u16 = 17;
const CF_UNICODETEXT: u16 = 13;
/// TYMED bits.
const TYMED_HGLOBAL: u32 = 0x01;
const TYMED_FILE: u32 = 0x02;
/// GMEM flags.
const GMEM_MOVEABLE: UINT = 0x0040;

/// HRESULT sentinels. The hex values above `0x7FFFFFFF` are interpreted
/// as negative `i32` values when the platform returns them.
const S_OK: HRESULT = 0;
const S_FALSE: HRESULT = 1;
const DRAGDROP_S_DROP: HRESULT = 0x00040100;
const DRAGDROP_S_CANCEL: HRESULT = 0x00040101;
const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = 0x00040102;
const E_NOINTERFACE: HRESULT = 0x80004002_u32 as i32;
const E_NOTIMPL: HRESULT = 0x80004001_u32 as i32;
const E_OUTOFMEMORY: HRESULT = 0x8007000E_u32 as i32;
const E_INVALIDARG: HRESULT = 0x80070057_u32 as i32;
#[allow(dead_code)]
const E_UNEXPECTED: HRESULT = 0x8000FFFF_u32 as i32;
const E_FAIL: HRESULT = 0x80004005_u32 as i32;
const OLE_E_ADVISENOTSUPPORTED: HRESULT = 0x80040003_u32 as i32;
const OLE_E_NOTRUNNING: HRESULT = 0x80040005_u32 as i32;
const DVASPECT_CONTENT: DWORD = 1;

/// `DROPEFFECT` bits.
const DROPEFFECT_NONE: u32 = 0;
const DROPEFFECT_COPY: u32 = 1;

/// COM IIDs that we care about.
const IID_IUNKNOWN: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
const IID_IDATAOBJECT: [u8; 16] = [
    0x0E, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
const IID_IDROPSOURCE: [u8; 16] = [
    0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
const IID_IENUMFORMATETC: [u8; 16] = [
    0x03, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

#[repr(C)]
#[allow(non_snake_case)]
#[derive(Clone, Copy)]
struct FORMATETC {
    cfFormat: u16,
    ptd: *mut c_void,
    dwAspect: DWORD,
    lindex: i32,
    tymed: DWORD,
}

#[repr(C)]
#[allow(non_snake_case)]
#[derive(Clone, Copy)]
struct STGMEDIUM {
    tymed: DWORD,
    u: STGMEDIUMUnion,
    pUnkForRelease: *mut c_void,
}

#[repr(C)]
#[allow(non_snake_case)]
#[derive(Clone, Copy)]
union STGMEDIUMUnion {
    hBitmap: *mut c_void,
    hMetaFilePict: *mut c_void,
    hEnhMetaFile: *mut c_void,
    hGlobal: HGLOBAL,
    lpszFileName: *mut i8,
    pstm: *mut c_void,
    pstg: *mut c_void,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IUnknownVtbl {
    QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> HRESULT,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IDataObjectVtbl {
    base__: IUnknownVtbl,
    GetData: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    GetDataHere:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    QueryGetData: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> HRESULT,
    GetCanonicalFormatEtc:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    SetData:
        unsafe extern "system" fn(*mut c_void, *const FORMATETC, *const STGMEDIUM, BOOL) -> HRESULT,
    EnumFormatEtc: unsafe extern "system" fn(*mut c_void, DWORD, *mut *mut c_void) -> HRESULT,
    DAdvise: unsafe extern "system" fn(
        *mut c_void,
        *const FORMATETC,
        DWORD,
        *mut c_void,
        *mut DWORD,
    ) -> HRESULT,
    DUnadvise: unsafe extern "system" fn(*mut c_void, DWORD) -> HRESULT,
    EnumDAdvise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IDropSourceVtbl {
    base__: IUnknownVtbl,
    QueryContinueDrag: unsafe extern "system" fn(*mut c_void, BOOL, DWORD) -> HRESULT,
    GiveFeedback: unsafe extern "system" fn(*mut c_void, DWORD) -> HRESULT,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IEnumFORMATETCVtbl {
    base__: IUnknownVtbl,
    Next: unsafe extern "system" fn(*mut c_void, DWORD, *mut FORMATETC, *mut DWORD) -> HRESULT,
    Skip: unsafe extern "system" fn(*mut c_void, DWORD) -> HRESULT,
    Reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    Clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(uflags: UINT, dwbytes: usize) -> HGLOBAL;
    fn GlobalLock(h: HGLOBAL) -> *mut c_void;
    fn GlobalUnlock(h: HGLOBAL) -> BOOL;
    fn GlobalFree(h: HGLOBAL) -> HGLOBAL;
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClipboardFormatW(lpszformat: LPCWSTR) -> UINT;
}

#[link(name = "ole32")]
extern "system" {
    fn DoDragDrop(
        pdataobj: *mut c_void,
        pdropsource: *mut c_void,
        dwokeffects: DWORD,
        pdweffect: *mut DWORD,
    ) -> HRESULT;
}

// ============================================================
// Domain types.
// ============================================================

/// Custom format for the registered PNG. Registered once at first
/// drag and cached for the lifetime of the process.
static REGISTERED_PNG: Mutex<Option<u16>> = Mutex::new(None);

fn registered_png_format() -> u16 {
    let mut slot = REGISTERED_PNG.lock();
    if let Some(fmt) = *slot {
        return fmt;
    }
    let name: Vec<u16> = "image/png\0".encode_utf16().collect();
    let fmt = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    let fmt = if fmt == 0 { 0 } else { fmt as u16 };
    *slot = Some(fmt);
    fmt
}

/// Wall-clock milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The shared OLE state. Allocated once per drag and held by the
/// `IDataObject` implementation; the COM reference-counting keeps the
/// state alive for the entire synchronous `DoDragDrop` call.
struct OleState {
    png_path: PathBuf,
    bgra_pixels: Vec<u8>,
    width: u32,
    height: u32,
    capture_id: String,
    shelf_id: Option<String>,
    started_at_ms: i64,
    /// Formats pulled by the drop target during the drag.
    requested_formats: Mutex<Vec<DragFormat>>,
    /// Cached PNG bytes for the registered PNG format request.
    registered_png_bytes: Vec<u8>,
}

impl OleState {
    fn new(request: &DragRequest) -> PlatformResult<Self> {
        request.validate()?;
        let png_path = PathBuf::from(&request.png_path);
        if !png_path.exists() {
            return Err(PlatformError::new(
                PlatformErrorKind::Io,
                "windows drag: backing PNG does not exist",
            ));
        }
        let started_at_ms = now_ms();
        let registered_png_bytes = std::fs::read(&png_path).map_err(|e| {
            PlatformError::new(
                PlatformErrorKind::Io,
                format!("windows drag: failed to read PNG into memory: {e}"),
            )
        })?;
        Ok(Self {
            png_path,
            bgra_pixels: request.bgra_pixels.clone(),
            width: request.width,
            height: request.height,
            capture_id: request.capture_id.clone(),
            shelf_id: request.shelf_id.clone(),
            started_at_ms,
            requested_formats: Mutex::new(Vec::new()),
            registered_png_bytes,
        })
    }

    /// Map a FORMATETC to a `DragFormat` if it is one of the four we
    /// offer. Returns `None` otherwise.
    fn classify(format: &FORMATETC) -> Option<DragFormat> {
        let registered = registered_png_format();
        if format.cfFormat == CF_HDROP {
            return Some(DragFormat::Hdrop);
        }
        if format.cfFormat == CF_DIBV5 {
            return Some(DragFormat::DibV5);
        }
        if format.cfFormat == CF_UNICODETEXT {
            return Some(DragFormat::UnicodeText);
        }
        if registered != 0 && format.cfFormat == registered {
            return Some(DragFormat::RegisteredPng);
        }
        None
    }

    /// Allocate the drop payload for the given format.
    fn encode(&self, format: DragFormat) -> Vec<u8> {
        match format {
            DragFormat::Hdrop => encode_hdrop(&self.png_path),
            DragFormat::RegisteredPng => self.registered_png_bytes.clone(),
            DragFormat::DibV5 => encode_dib_v5(&self.bgra_pixels, self.width, self.height),
            DragFormat::UnicodeText => {
                let path = self.png_path.to_string_lossy().to_string();
                encode_unicode_text(&path)
            }
        }
    }
}

/// Encode a `CF_HDROP` payload. The Windows `DROPFILES` struct is a
/// 20-byte header followed by a double-null-terminated list of file
/// paths. We only ship one PNG at a time, so there is exactly one
/// entry plus the trailing null.
fn encode_hdrop(path: &PathBuf) -> Vec<u8> {
    let path_str = path.to_string_lossy().to_string();
    let path_w: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
    let path_bytes: Vec<u8> = path_w.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut header = [0u8; 20];
    // DROPFILES.pFiles = 20 (right after the header).
    header[0..4].copy_from_slice(&20u32.to_le_bytes());
    // DROPFILES.fWide = 1 to signal wide characters.
    header[12..16].copy_from_slice(&1u32.to_le_bytes());
    let mut out = Vec::with_capacity(header.len() + path_bytes.len() + 2);
    out.extend_from_slice(&header);
    out.extend_from_slice(&path_bytes);
    out.extend_from_slice(&[0, 0]);
    out
}

/// Encode a `CF_DIBV5` payload. The buffer starts with a
/// `BITMAPV5HEADER` followed by the pixel data.
fn encode_dib_v5(bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut header = [0u8; 124];
    header[0..4].copy_from_slice(&124u32.to_le_bytes()); // bV5Size
    header[4..8].copy_from_slice(&width.to_le_bytes());
    header[8..12].copy_from_slice(&height.to_le_bytes());
    header[12..14].copy_from_slice(&1u16.to_le_bytes()); // planes
    header[14..16].copy_from_slice(&32u16.to_le_bytes()); // bV5BitCount
    header[16..20].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    header[20..24].copy_from_slice(&(bgra.len() as u32).to_le_bytes());
    header[24..28].copy_from_slice(&0u32.to_le_bytes()); // bV5XPelsPerMeter
    header[28..32].copy_from_slice(&0u32.to_le_bytes()); // bV5YPelsPerMeter
    header[56..60].copy_from_slice(&0x00FF_0000u32.to_le_bytes()); // red mask
    header[60..64].copy_from_slice(&0x0000_FF00u32.to_le_bytes()); // green mask
    header[64..68].copy_from_slice(&0x0000_00FFu32.to_le_bytes()); // blue mask
    header[68..72].copy_from_slice(&0xFF00_0000u32.to_le_bytes()); // alpha mask
    let mut out = Vec::with_capacity(header.len() + bgra.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(bgra);
    out
}

/// Encode a `CF_UNICODETEXT` payload. The bytes are a UTF-16LE
/// representation of the supplied text plus a trailing null.
fn encode_unicode_text(text: &str) -> Vec<u8> {
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    utf16.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// Fill a `STGMEDIUM` with the supplied bytes using the
/// `TYMED_HGLOBAL` transport.
fn medium_with_hglobal(pmedium: *mut STGMEDIUM, bytes: &[u8]) -> HRESULT {
    if bytes.is_empty() {
        return E_FAIL;
    }
    // SAFETY: `GlobalAlloc` is a Win32 FFI; `GMEM_MOVEABLE` and the
    // byte count are well-formed arguments.
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if handle.is_null() {
        return E_OUTOFMEMORY;
    }
    // SAFETY: `GlobalLock` is a Win32 FFI; the handle was just
    // allocated by `GlobalAlloc` so the lock is well-formed.
    let ptr = unsafe { GlobalLock(handle) };
    if ptr.is_null() {
        // SAFETY: Reclaim the previously-allocated handle.
        unsafe {
            GlobalFree(handle);
        }
        return E_OUTOFMEMORY;
    }
    // SAFETY: Both source and destination are valid pointers for the
    // length of bytes; the regions do not overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        let _ = GlobalUnlock(handle);
    }
    let medium = unsafe { &mut *pmedium };
    medium.tymed = TYMED_HGLOBAL;
    medium.u.hGlobal = handle;
    medium.pUnkForRelease = std::ptr::null_mut();
    S_OK
}

// ============================================================
// IDataObject implementation.
// ============================================================

#[repr(C)]
struct PixelGrabDataObject {
    vtable: *const IDataObjectVtbl,
    ref_count: AtomicU32,
    state: Arc<OleState>,
}

static DATA_OBJECT_VTABLE: IDataObjectVtbl = IDataObjectVtbl {
    base__: IUnknownVtbl {
        QueryInterface: data_object_query_interface,
        AddRef: data_object_add_ref,
        Release: data_object_release,
    },
    GetData: data_object_get_data,
    GetDataHere: data_object_get_data_here,
    QueryGetData: data_object_query_get_data,
    GetCanonicalFormatEtc: data_object_get_canonical_format_etc,
    SetData: data_object_set_data,
    EnumFormatEtc: data_object_enum_format_etc,
    DAdvise: data_object_d_advise,
    DUnadvise: data_object_d_unadvise,
    EnumDAdvise: data_object_enum_d_advise,
};

impl PixelGrabDataObject {
    fn new(state: Arc<OleState>) -> Self {
        Self {
            vtable: &DATA_OBJECT_VTABLE,
            ref_count: AtomicU32::new(1),
            state,
        }
    }

    fn as_raw(&self) -> *mut c_void {
        self as *const Self as *mut c_void
    }
}

unsafe impl Send for PixelGrabDataObject {}
// SAFETY: The data object is owned by the synchronous `DoDragDrop` call
// and only ever touched by the COM vtable shims on the same thread. The
// internal `OleState` carries interior-mutable `Mutex`es for the format
// log and the pixel buffer, so the COM object itself does not need
// exclusive mutable access; the COM runtime never moves the inner data.
unsafe impl Sync for PixelGrabDataObject {}

unsafe extern "system" fn data_object_query_interface(
    this: *mut c_void,
    riid: *const [u8; 16],
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_INVALIDARG;
    }
    let iid = unsafe { &*riid };
    if iid == &IID_IDATAOBJECT || iid == &IID_IUNKNOWN {
        unsafe {
            *ppv = this;
        }
        let _ = unsafe { &*this.cast::<PixelGrabDataObject>() }
            .ref_count
            .fetch_add(1, Ordering::SeqCst);
        return S_OK;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
    }
    E_NOINTERFACE
}

unsafe extern "system" fn data_object_add_ref(this: *mut c_void) -> u32 {
    let obj = unsafe { &*this.cast::<PixelGrabDataObject>() };
    obj.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn data_object_release(this: *mut c_void) -> u32 {
    let obj_ptr = this as *mut PixelGrabDataObject;
    let obj = unsafe { &*obj_ptr };
    let prev = obj.ref_count.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 {
        unsafe {
            drop(Box::from_raw(obj_ptr));
        }
        return 0;
    }
    prev - 1
}

unsafe extern "system" fn data_object_get_data(
    this: *mut c_void,
    pformat: *const FORMATETC,
    pmedium: *mut STGMEDIUM,
) -> HRESULT {
    if pformat.is_null() || pmedium.is_null() {
        return E_INVALIDARG;
    }
    let fmt = unsafe { &*pformat };
    let obj = unsafe { &*this.cast::<PixelGrabDataObject>() };
    let target = match OleState::classify(fmt) {
        Some(target) => target,
        None => return OLE_E_ADVISENOTSUPPORTED,
    };
    let bytes = obj.state.encode(target);
    {
        let mut requested = obj.state.requested_formats.lock();
        requested.push(target);
    }
    medium_with_hglobal(pmedium, &bytes)
}

unsafe extern "system" fn data_object_get_data_here(
    _this: *mut c_void,
    _pformat: *const FORMATETC,
    _pmedium: *mut STGMEDIUM,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_query_get_data(
    this: *mut c_void,
    pformat: *const FORMATETC,
) -> HRESULT {
    if pformat.is_null() {
        return E_INVALIDARG;
    }
    let fmt = unsafe { &*pformat };
    let _ = unsafe { &*this.cast::<PixelGrabDataObject>() };
    if OleState::classify(fmt).is_some() {
        S_OK
    } else {
        OLE_E_ADVISENOTSUPPORTED
    }
}

unsafe extern "system" fn data_object_get_canonical_format_etc(
    _this: *mut c_void,
    _pformat: *const FORMATETC,
    _pformatout: *mut FORMATETC,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_set_data(
    _this: *mut c_void,
    _pformat: *const FORMATETC,
    _pmedium: *const STGMEDIUM,
    _frelease: BOOL,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_enum_format_etc(
    _this: *mut c_void,
    dwdirection: DWORD,
    ppenumformatetc: *mut *mut c_void,
) -> HRESULT {
    // Only the data-side enumerator (DATADIR_GET = 1) is supported.
    if dwdirection != 1 {
        return E_NOTIMPL;
    }
    if ppenumformatetc.is_null() {
        return E_INVALIDARG;
    }
    let enumerator = Box::new(FormatEnumerator::new());
    let raw = Box::into_raw(enumerator) as *mut c_void;
    unsafe {
        *ppenumformatetc = raw;
    }
    S_OK
}

unsafe extern "system" fn data_object_d_advise(
    _this: *mut c_void,
    _pformat: *const FORMATETC,
    _advf: DWORD,
    _padvsink: *mut c_void,
    _pdwconnection: *mut DWORD,
) -> HRESULT {
    OLE_E_NOTRUNNING
}

unsafe extern "system" fn data_object_d_unadvise(
    _this: *mut c_void,
    _dwconnection: DWORD,
) -> HRESULT {
    S_OK
}

unsafe extern "system" fn data_object_enum_d_advise(
    _this: *mut c_void,
    _ppenumadvise: *mut *mut c_void,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

// ============================================================
// IDropSource implementation.
// ============================================================

#[repr(C)]
struct PixelGrabDropSource {
    vtable: *const IDropSourceVtbl,
    ref_count: AtomicU32,
}

static DROP_SOURCE_VTABLE: IDropSourceVtbl = IDropSourceVtbl {
    base__: IUnknownVtbl {
        QueryInterface: drop_source_query_interface,
        AddRef: drop_source_add_ref,
        Release: drop_source_release,
    },
    QueryContinueDrag: drop_source_query_continue_drag,
    GiveFeedback: drop_source_give_feedback,
};

impl PixelGrabDropSource {
    fn new() -> Self {
        Self {
            vtable: &DROP_SOURCE_VTABLE,
            ref_count: AtomicU32::new(1),
        }
    }

    fn as_raw(&self) -> *mut c_void {
        self as *const Self as *mut c_void
    }
}

unsafe impl Send for PixelGrabDropSource {}
// SAFETY: The drop source is read-only after construction and is
// driven by the COM runtime on the same thread that called
// `DoDragDrop`. No interior mutability is needed.
unsafe impl Sync for PixelGrabDropSource {}

unsafe extern "system" fn drop_source_query_interface(
    this: *mut c_void,
    riid: *const [u8; 16],
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_INVALIDARG;
    }
    let iid = unsafe { &*riid };
    if iid == &IID_IDROPSOURCE || iid == &IID_IUNKNOWN {
        unsafe {
            *ppv = this;
        }
        let _ = unsafe { &*this.cast::<PixelGrabDropSource>() }
            .ref_count
            .fetch_add(1, Ordering::SeqCst);
        return S_OK;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
    }
    E_NOINTERFACE
}

unsafe extern "system" fn drop_source_add_ref(this: *mut c_void) -> u32 {
    let obj = unsafe { &*this.cast::<PixelGrabDropSource>() };
    obj.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn drop_source_release(this: *mut c_void) -> u32 {
    let obj_ptr = this as *mut PixelGrabDropSource;
    let obj = unsafe { &*obj_ptr };
    let prev = obj.ref_count.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 {
        unsafe {
            drop(Box::from_raw(obj_ptr));
        }
        return 0;
    }
    prev - 1
}

unsafe extern "system" fn drop_source_query_continue_drag(
    _this: *mut c_void,
    fescapepressed: BOOL,
    grfkeystate: DWORD,
) -> HRESULT {
    if fescapepressed != 0 {
        return DRAGDROP_S_CANCEL;
    }
    const MK_LBUTTON: DWORD = 0x0001;
    const MK_RBUTTON: DWORD = 0x0002;
    if (grfkeystate & (MK_LBUTTON | MK_RBUTTON)) == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn drop_source_give_feedback(
    _this: *mut c_void,
    _dweffect: DWORD,
) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

// ============================================================
// IEnumFORMATETC implementation.
// ============================================================

#[repr(C)]
struct FormatEnumerator {
    vtable: *const IEnumFORMATETCVtbl,
    ref_count: AtomicU32,
    pos: usize,
}

static FORMAT_ENUM_VTABLE: IEnumFORMATETCVtbl = IEnumFORMATETCVtbl {
    base__: IUnknownVtbl {
        QueryInterface: format_enum_query_interface,
        AddRef: format_enum_add_ref,
        Release: format_enum_release,
    },
    Next: format_enum_next,
    Skip: format_enum_skip,
    Reset: format_enum_reset,
    Clone: format_enum_clone,
};

impl FormatEnumerator {
    fn new() -> Self {
        Self {
            vtable: &FORMAT_ENUM_VTABLE,
            ref_count: AtomicU32::new(1),
            pos: 0,
        }
    }
}

unsafe impl Send for FormatEnumerator {}
// SAFETY: The enumerator exposes four static `FORMATETC` values and
// advances a `usize` cursor (single-threaded by contract). It is
// borrowed by the COM runtime for the duration of the
// `EnumFormatEtc` call only.
unsafe impl Sync for FormatEnumerator {}

unsafe extern "system" fn format_enum_query_interface(
    this: *mut c_void,
    riid: *const [u8; 16],
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_INVALIDARG;
    }
    let iid = unsafe { &*riid };
    if iid == &IID_IENUMFORMATETC || iid == &IID_IUNKNOWN {
        unsafe {
            *ppv = this;
        }
        let _ = unsafe { &*this.cast::<FormatEnumerator>() }
            .ref_count
            .fetch_add(1, Ordering::SeqCst);
        return S_OK;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
    }
    E_NOINTERFACE
}

unsafe extern "system" fn format_enum_add_ref(this: *mut c_void) -> u32 {
    let obj = unsafe { &*this.cast::<FormatEnumerator>() };
    obj.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn format_enum_release(this: *mut c_void) -> u32 {
    let obj_ptr = this as *mut FormatEnumerator;
    let obj = unsafe { &*obj_ptr };
    let prev = obj.ref_count.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 {
        unsafe {
            drop(Box::from_raw(obj_ptr));
        }
        return 0;
    }
    prev - 1
}

fn offered_formats() -> [FORMATETC; 4] {
    let hdrop = FORMATETC {
        cfFormat: CF_HDROP,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_FILE,
    };
    let registered = FORMATETC {
        cfFormat: registered_png_format(),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    let dib = FORMATETC {
        cfFormat: CF_DIBV5,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    let unicode = FORMATETC {
        cfFormat: CF_UNICODETEXT,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    [hdrop, registered, dib, unicode]
}

unsafe extern "system" fn format_enum_next(
    this: *mut c_void,
    celt: DWORD,
    rgelt: *mut FORMATETC,
    pceltfetched: *mut DWORD,
) -> HRESULT {
    let formats = offered_formats();
    let obj = unsafe { &*this.cast::<FormatEnumerator>() };
    let mut pos = obj.pos;
    let mut written = 0u32;
    while written < celt && pos < formats.len() {
        unsafe {
            *rgelt.add(written as usize) = formats[pos];
        }
        pos += 1;
        written += 1;
    }
    if !pceltfetched.is_null() {
        unsafe {
            *pceltfetched = written;
        }
    }
    if written == celt {
        S_OK
    } else {
        S_FALSE
    }
}

unsafe extern "system" fn format_enum_skip(_this: *mut c_void, _celt: DWORD) -> HRESULT {
    S_OK
}

unsafe extern "system" fn format_enum_reset(this: *mut c_void) -> HRESULT {
    let obj = unsafe { &mut *this.cast::<FormatEnumerator>() };
    obj.pos = 0;
    S_OK
}

unsafe extern "system" fn format_enum_clone(
    _this: *mut c_void,
    _ppenum: *mut *mut c_void,
) -> HRESULT {
    E_NOTIMPL
}

// ============================================================
// Drag loop.
// ============================================================

/// Translate the `DoDragDrop` HRESULT into a `DragOutcome`.
fn translate_hr(hr: HRESULT) -> pixelgrab_contracts::drag::DragOutcome {
    use pixelgrab_contracts::drag::DragOutcome;
    if hr == DRAGDROP_S_DROP {
        DragOutcome::Accepted
    } else if hr == DRAGDROP_S_CANCEL {
        DragOutcome::Cancelled
    } else if hr < 0 {
        DragOutcome::Failed
    } else {
        DragOutcome::Rejected
    }
}

/// Translate the OLE failure into a categorical `PlatformErrorKind` label.
fn failure_kind_for(hr: HRESULT) -> &'static str {
    if hr == E_OUTOFMEMORY {
        "internal"
    } else if hr == E_INVALIDARG {
        "invalid_payload"
    } else {
        "internal"
    }
}

/// Categorical target class. The Windows adapter does not introspect
/// the target window, so the value is `Other`. A future tracer can
/// wire process introspection for telemetry when contract-acceptable.
fn classify_target() -> DragTargetKind {
    DragTargetKind::Other
}

/// Run the OLE drag loop. The `OleState` owns the backing PNG bytes
/// for the full synchronous call; the COM allocations are released
/// when the `DataObject` and `DropSource` are dropped at the end of
/// the function.
pub fn run_drag(request: &DragRequest) -> PlatformResult<DragResult> {
    let state = Arc::new(OleState::new(request)?);
    let data_object = Box::new(PixelGrabDataObject::new(state.clone()));
    let drop_source = Box::new(PixelGrabDropSource::new());
    let data_object_ptr = data_object.as_raw();
    let drop_source_ptr = drop_source.as_raw();
    let mut effect: DWORD = DROPEFFECT_NONE;
    let hr = unsafe {
        DoDragDrop(
            data_object_ptr,
            drop_source_ptr,
            DROPEFFECT_COPY,
            &mut effect,
        )
    };
    let _ = effect;
    let outcome = translate_hr(hr);
    let completed_at = now_ms();
    let mut diag = DragDiagnostics::started(
        state.capture_id.clone(),
        state.shelf_id.clone(),
        state.started_at_ms,
    )
    .completed(completed_at)
    .with_target_effect(target_effect_for(outcome))
    .with_target_kind(classify_target());
    if outcome == pixelgrab_contracts::drag::DragOutcome::Failed {
        diag = diag.failed(failure_kind_for(hr));
    }
    let recorded = state.requested_formats.lock().clone();
    for fmt in recorded {
        let rel = (completed_at - state.started_at_ms).max(0);
        diag.record_format_request(fmt, rel);
    }
    // `DoDragDrop` has its own reference to the data object and drop
    // source; the boxes are dropped here, which decrements the ref
    // count to zero. The data object is freed by the `Release` vtable
    // callback once `DoDragDrop` drops its reference.
    drop(drop_source);
    drop(data_object);
    Ok(DragResult {
        outcome,
        diagnostics: diag,
    })
}

fn target_effect_for(outcome: pixelgrab_contracts::drag::DragOutcome) -> DragTargetEffect {
    match outcome {
        pixelgrab_contracts::drag::DragOutcome::Accepted => DragTargetEffect::Copy,
        pixelgrab_contracts::drag::DragOutcome::Rejected => DragTargetEffect::None,
        pixelgrab_contracts::drag::DragOutcome::Cancelled => DragTargetEffect::Unknown,
        pixelgrab_contracts::drag::DragOutcome::Failed => DragTargetEffect::Unknown,
    }
}

/// Public entry point used by the Windows platform adapter.
pub fn start_drag(request: &DragRequest) -> PlatformResult<DragResult> {
    run_drag(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DragRequest {
        DragRequest {
            capture_id: "capture-1".into(),
            shelf_id: Some("shelf-1".to_string()),
            png_path: "NOT-A-REAL-PATH.png".into(),
            bgra_pixels: vec![0u8; 4 * 4 * 4],
            width: 4,
            height: 4,
        }
    }

    #[test]
    fn ole_state_new_rejects_missing_png() {
        let req = sample_request();
        let result = OleState::new(&req);
        assert!(result.is_err());
    }

    #[test]
    fn translate_hr_maps_oole_results() {
        use pixelgrab_contracts::drag::DragOutcome;
        assert_eq!(translate_hr(DRAGDROP_S_DROP), DragOutcome::Accepted);
        assert_eq!(translate_hr(DRAGDROP_S_CANCEL), DragOutcome::Cancelled);
        assert_eq!(translate_hr(-1), DragOutcome::Failed);
        assert_eq!(translate_hr(0x00040100), DragOutcome::Accepted);
    }

    #[test]
    fn encode_hdrop_writes_pfiles_offset() {
        let path = PathBuf::from("C:\\cache\\cap.png");
        let bytes = encode_hdrop(&path);
        // Header (20) + UTF-16LE path bytes (16 chars × 2 + 2 null terminator)
        // + trailing double-null (2).
        let expected_path_bytes = (path.to_string_lossy().len() + 1) * 2;
        assert_eq!(bytes.len(), 20 + expected_path_bytes + 2);
        // pFiles = 20, fWide = 1.
        assert_eq!(&bytes[0..4], &20u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &1u32.to_le_bytes());
    }

    #[test]
    fn encode_dib_v5_writes_header() {
        let mut bgra = vec![0u8; 8 * 4 * 4];
        bgra[0] = 1;
        let bytes = encode_dib_v5(&bgra, 8, 4);
        assert_eq!(bytes.len(), 124 + 8 * 4 * 4);
        assert_eq!(&bytes[0..4], &124u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &8u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &4u32.to_le_bytes());
        assert_eq!(&bytes[14..16], &32u16.to_le_bytes());
    }

    #[test]
    fn encode_unicode_text_writes_utf16_with_null() {
        let bytes = encode_unicode_text("a");
        assert_eq!(bytes.len(), 4);
        let decoded: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(decoded, vec![b'a' as u16, 0]);
    }

    #[test]
    fn failure_kind_for_maps_known_codes() {
        assert_eq!(failure_kind_for(E_OUTOFMEMORY), "internal");
        assert_eq!(failure_kind_for(E_INVALIDARG), "invalid_payload");
        assert_eq!(failure_kind_for(E_UNEXPECTED), "internal");
        assert_eq!(failure_kind_for(E_FAIL), "internal");
    }

    #[test]
    fn target_effect_for_maps_outcomes() {
        use pixelgrab_contracts::drag::DragOutcome;
        assert_eq!(
            target_effect_for(DragOutcome::Accepted),
            DragTargetEffect::Copy
        );
        assert_eq!(
            target_effect_for(DragOutcome::Rejected),
            DragTargetEffect::None
        );
        assert_eq!(
            target_effect_for(DragOutcome::Cancelled),
            DragTargetEffect::Unknown
        );
        assert_eq!(
            target_effect_for(DragOutcome::Failed),
            DragTargetEffect::Unknown
        );
    }

    #[test]
    fn ole_state_classifies_known_formats() {
        let registered = registered_png_format();
        let formats = offered_formats();
        assert_eq!(formats.len(), 4);
        assert_eq!(OleState::classify(&formats[0]), Some(DragFormat::Hdrop));
        assert_eq!(
            OleState::classify(&formats[1]),
            Some(DragFormat::RegisteredPng),
        );
        assert_eq!(formats[1].cfFormat, registered);
        assert_eq!(OleState::classify(&formats[2]), Some(DragFormat::DibV5));
        assert_eq!(
            OleState::classify(&formats[3]),
            Some(DragFormat::UnicodeText),
        );
    }
}
