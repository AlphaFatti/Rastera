//! Ownership boundary: AppKit handle stays on main; logger handle is C++ shared state.
use std::{
    ffi::{c_void, CString},
    io,
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
};
unsafe extern "C" {
    fn rastera_native_log(line: *const std::ffi::c_char);
    fn rastera_native_create(stats: *mut *mut c_void, disable_animations: bool) -> *mut c_void;
    fn rastera_native_destroy(handle: *mut c_void);
    fn rastera_native_is_open(handle: *mut c_void) -> bool;
    fn rastera_native_report(stats: *mut c_void);
    fn rastera_native_stats_destroy(stats: *mut c_void);
}
pub struct NativeOutput {
    handle: NonNull<c_void>,
    _main_only: PhantomData<Rc<()>>,
}
pub struct NativeStats(NonNull<c_void>);
// SAFETY: This independent handle owns a C++ shared_ptr to mutex/atomic-protected
// telemetry only. It neither owns nor accesses any AppKit/Objective-C object.
unsafe impl Send for NativeStats {}
impl NativeOutput {
    /// Called from the main-thread eframe creation callback only.
    pub fn new(disable_animations: bool) -> io::Result<(Self, NativeStats)> {
        let mut stats = std::ptr::null_mut();
        // SAFETY: out pointer is valid; native API creates an owned main-thread object.
        let handle = unsafe { rastera_native_create(&mut stats, disable_animations) };
        let output = Self {
            handle: NonNull::new(handle).ok_or_else(|| {
                io::Error::other("native Metal output initialization failed; requires macOS 14+")
            })?,
            _main_only: PhantomData,
        };
        let stats = NonNull::new(stats)
            .ok_or_else(|| io::Error::other("native telemetry initialization failed"))?;
        Ok((output, NativeStats(stats)))
    }
    pub fn is_open(&self) -> bool {
        // SAFETY: handle remains owned, and this !Send type stays on the main thread.
        unsafe { rastera_native_is_open(self.handle.as_ptr()) }
    }
}
impl Drop for NativeOutput {
    fn drop(&mut self) {
        // SAFETY: unique main-thread owner. Native shutdown invalidates/joins the
        // render loop before detaching the view; GPU callbacks retain their own state.
        unsafe { rastera_native_destroy(self.handle.as_ptr()) }
    }
}
impl NativeStats {
    pub fn report(&self) {
        // SAFETY: valid telemetry-only shared_ptr, drained under its C++ mutex.
        unsafe { rastera_native_report(self.0.as_ptr()) }
    }
}
impl Drop for NativeStats {
    fn drop(&mut self) {
        // SAFETY: releases this unique shared_ptr wrapper, not the underlying
        // state still retained by the presenter or Metal callbacks.
        unsafe { rastera_native_stats_destroy(self.0.as_ptr()) }
    }
}

/// All native-control text shares C stdio's FILE lock, including clock anchors.
pub fn log(line: String) {
    let line = CString::new(line).expect("diagnostic message cannot contain NUL");
    // SAFETY: NUL-terminated string stays alive for this synchronous fputs call.
    unsafe { rastera_native_log(line.as_ptr()) }
}
