//! Native Windows clipboard backend using Win32 API.
//!
//! Provides event-driven clipboard monitoring via `AddClipboardFormatListener`,
//! contention handling with exponential backoff, and full UTF-16 Unicode text support (`CF_UNICODETEXT`).

use crate::backend::ClipboardBackend;
use crate::error::ClipboardError;
use crate::types::ClipboardContent;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::{debug, error, info};

use windows_sys::Win32::Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
    IsClipboardFormatAvailable, OpenClipboard, RemoveClipboardFormatListener, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    RegisterClassW, TranslateMessage, UnregisterClassW, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE,
    WM_CLOSE, WM_DESTROY, WNDCLASSW,
};

/// Win32 clipboard format constant for Unicode text.
const CF_UNICODETEXT: u32 = 13;

/// Maximum retry attempts when acquiring Windows clipboard during contention.
const CLIPBOARD_RETRY_COUNT: u32 = 10;
/// Base retry delay in milliseconds for exponential backoff.
const BASE_RETRY_DELAY_MS: u64 = 10;
/// Broadcast channel capacity for clipboard change notifications.
const DEFAULT_CHANNEL_CAPACITY: usize = 64;

// FFI declaration for GlobalFree from kernel32.dll
extern "system" {
    fn GlobalFree(hmem: HGLOBAL) -> HGLOBAL;
}

/// RAII guard ensuring `CloseClipboard` is always called when clipboard is opened.
struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: CloseClipboard is safe to call when OpenClipboard succeeded.
        unsafe {
            CloseClipboard();
        }
    }
}

/// Helper function to open Windows clipboard with retry backoff to handle contention.
fn open_clipboard_with_retry() -> Result<ClipboardGuard, ClipboardError> {
    for attempt in 0..CLIPBOARD_RETRY_COUNT {
        // SAFETY: OpenClipboard(null_mut()) associates clipboard access with the current task/thread.
        let ok = unsafe { OpenClipboard(std::ptr::null_mut()) };
        if ok != 0 {
            return Ok(ClipboardGuard);
        }

        let delay = BASE_RETRY_DELAY_MS * (1 << attempt.min(5));
        debug!(
            attempt,
            delay_ms = delay,
            "Clipboard busy; retrying with backoff"
        );
        std::thread::sleep(Duration::from_millis(delay));
    }

    Err(ClipboardError::BackendError(
        "Failed to open Windows clipboard: access locked by another application".to_string(),
    ))
}

/// Native Windows implementation of `ClipboardBackend`.
pub struct WindowsClipboardBackend {
    notifier: broadcast::Sender<ClipboardContent>,
    hwnd: Arc<AtomicIsize>,
    is_running: Arc<AtomicBool>,
    sync_lock: Arc<Mutex<()>>,
    listener_thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for WindowsClipboardBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsClipboardBackend")
            .field("hwnd", &self.hwnd.load(Ordering::Relaxed))
            .field("is_running", &self.is_running.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl WindowsClipboardBackend {
    /// Creates a new `WindowsClipboardBackend` and starts the background Win32 message loop listener.
    pub fn new() -> Result<Self, ClipboardError> {
        let (notifier, _) = broadcast::channel(DEFAULT_CHANNEL_CAPACITY);
        let hwnd = Arc::new(AtomicIsize::new(0));
        let is_running = Arc::new(AtomicBool::new(true));
        let sync_lock = Arc::new(Mutex::new(()));

        let thread_tx = notifier.clone();
        let thread_hwnd = hwnd.clone();
        let thread_running = is_running.clone();
        let thread_sync = sync_lock.clone();

        // Spawn a dedicated OS thread running the Win32 message pump for event-driven clipboard updates
        let listener_thread = std::thread::Builder::new()
            .name("bridge-winclip-listener".to_string())
            .spawn(move || {
                run_clipboard_listener_loop(thread_tx, thread_hwnd, thread_running, thread_sync);
            })
            .map_err(|e| {
                ClipboardError::BackendError(format!("Failed to spawn listener thread: {e}"))
            })?;

        // Wait up to 1 second for the window to be created and registered
        let start = std::time::Instant::now();
        while hwnd.load(Ordering::Acquire) == 0 && start.elapsed() < Duration::from_millis(1000) {
            std::thread::sleep(Duration::from_millis(10));
        }

        info!("WindowsClipboardBackend initialized successfully with event-driven listener");

        Ok(Self {
            notifier,
            hwnd,
            is_running,
            sync_lock,
            listener_thread: Some(listener_thread),
        })
    }
}

impl Default for WindowsClipboardBackend {
    fn default() -> Self {
        Self::new().expect("Failed to initialize WindowsClipboardBackend")
    }
}

impl ClipboardBackend for WindowsClipboardBackend {
    fn get_content(&self) -> Result<Option<ClipboardContent>, ClipboardError> {
        let _process_lock = self.sync_lock.lock().unwrap();

        // SAFETY: Checking clipboard format availability before attempting to read
        let available = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) };
        if available == 0 {
            return Ok(None);
        }

        let _guard = open_clipboard_with_retry()?;

        // SAFETY: GetClipboardData with CF_UNICODETEXT returns an HGLOBAL handle if available
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
        if handle.is_null() {
            return Ok(None);
        }

        // SAFETY: GlobalLock returns a pointer to the UTF-16 character memory buffer
        let ptr = unsafe { GlobalLock(handle) } as *const u16;
        if ptr.is_null() {
            return Ok(None);
        }

        // SAFETY: GlobalSize returns the allocated size in bytes of the HGLOBAL handle
        let byte_size = unsafe { GlobalSize(handle) };
        let max_elements = byte_size / 2;

        let mut len = 0;
        while len < max_elements {
            // SAFETY: Dereferencing bounded offset within GlobalLock slice
            let ch = unsafe { *ptr.add(len) };
            if ch == 0 {
                break;
            }
            len += 1;
        }

        // SAFETY: Creating slice from verified valid, locked memory pointer
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        let text = String::from_utf16_lossy(slice);

        // SAFETY: Unlocking previously locked global memory handle
        unsafe {
            GlobalUnlock(handle);
        }

        Ok(Some(ClipboardContent::Text(text)))
    }

    fn set_content(&self, content: ClipboardContent) -> Result<(), ClipboardError> {
        let _process_lock = self.sync_lock.lock().unwrap();

        let text = match content {
            ClipboardContent::Text(t) => t,
            ClipboardContent::Html { html, .. } => html,
            ClipboardContent::Image { .. } => {
                return Err(ClipboardError::UnsupportedFormat(
                    "Image clipboard writing is not yet implemented in WindowsClipboardBackend"
                        .to_string(),
                ));
            }
        };

        let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_len = utf16.len() * 2;

        // SAFETY: Allocating movable memory buffer for the Windows clipboard
        let h_mem = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len) };
        if h_mem.is_null() {
            return Err(ClipboardError::BackendError(
                "Failed to allocate global memory for Windows clipboard".to_string(),
            ));
        }

        // SAFETY: GlobalLock provides writable pointer to the allocated memory
        let ptr = unsafe { GlobalLock(h_mem) }.cast::<u16>();
        if ptr.is_null() {
            // SAFETY: Free allocated memory if lock failed
            unsafe {
                GlobalFree(h_mem);
            }
            return Err(ClipboardError::BackendError(
                "Failed to lock allocated global memory".to_string(),
            ));
        }

        // SAFETY: Copying utf-16 slice into allocated buffer
        unsafe {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
            GlobalUnlock(h_mem);
        }

        let _guard = open_clipboard_with_retry()?;

        // SAFETY: EmptyClipboard clears the clipboard before writing new data
        let empty_ok = unsafe { EmptyClipboard() };
        if empty_ok == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            // SAFETY: Free memory on failure
            unsafe {
                GlobalFree(h_mem);
            }
            return Err(ClipboardError::BackendError(format!(
                "Failed to empty Windows clipboard (GetLastError={err})"
            )));
        }

        // SAFETY: SetClipboardData transfers ownership of h_mem to the system if successful
        let set_res = unsafe { SetClipboardData(CF_UNICODETEXT, h_mem) };
        if set_res.is_null() {
            // SAFETY: If SetClipboardData fails, the application retains ownership and must free
            unsafe {
                GlobalFree(h_mem);
            }
            return Err(ClipboardError::BackendError(
                "Failed to set Windows clipboard data".to_string(),
            ));
        }

        debug!(
            len = text.len(),
            "Updated local Windows clipboard with Unicode text"
        );
        Ok(())
    }

    fn clear(&self) -> Result<(), ClipboardError> {
        let _process_lock = self.sync_lock.lock().unwrap();
        let _guard = open_clipboard_with_retry()?;
        // SAFETY: EmptyClipboard clears the clipboard content
        let ok = unsafe { EmptyClipboard() };
        if ok == 0 {
            return Err(ClipboardError::BackendError(
                "Failed to clear Windows clipboard".to_string(),
            ));
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<ClipboardContent> {
        self.notifier.subscribe()
    }
}

impl Drop for WindowsClipboardBackend {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Release);
        let hwnd_val = self.hwnd.load(Ordering::Acquire);
        if hwnd_val != 0 {
            // SAFETY: Posting WM_CLOSE to the message-only listener window to trigger clean loop exit
            unsafe {
                PostMessageW(hwnd_val as HWND, WM_CLOSE, 0, 0);
            }
        }

        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Runs the Win32 message-only window and clipboard listener event loop on a dedicated thread.
#[allow(clippy::needless_pass_by_value)]
fn run_clipboard_listener_loop(
    tx: broadcast::Sender<ClipboardContent>,
    hwnd_out: Arc<AtomicIsize>,
    is_running: Arc<AtomicBool>,
    sync_lock: Arc<Mutex<()>>,
) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let class_name: Vec<u16> = format!("BridgeOS_Clipboard_Listener_{nanos}\0")
        .encode_utf16()
        .collect();

    // SAFETY: Initializing WNDCLASSW for our message-only window
    let wnd_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(listener_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: std::ptr::null_mut(),
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };

    // SAFETY: Registering the window class with the OS
    let class_atom = unsafe { RegisterClassW(&raw const wnd_class) };
    if class_atom == 0 {
        error!("Failed to register Win32 clipboard window class");
        return;
    }

    // SAFETY: Creating message-only window by setting parent to HWND_MESSAGE
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        error!("Failed to create Win32 message-only clipboard window");
        unsafe {
            UnregisterClassW(class_name.as_ptr(), std::ptr::null_mut());
        }
        return;
    }

    // SAFETY: Registering the message window with the OS clipboard format listener
    let listener_added = unsafe { AddClipboardFormatListener(hwnd) };
    if listener_added == 0 {
        error!("Failed to register clipboard format listener with Windows");
        unsafe {
            DestroyWindow(hwnd);
            UnregisterClassW(class_name.as_ptr(), std::ptr::null_mut());
        }
        return;
    }

    hwnd_out.store(hwnd as isize, Ordering::Release);
    debug!(hwnd = ?hwnd, "Registered Win32 clipboard format listener");

    // Enter message pump
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: GetMessageW blocks until a message is posted to this thread's message queue
    while is_running.load(Ordering::Acquire)
        && (unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0)
    {
        if msg.message == WM_CLIPBOARDUPDATE {
            debug!("Received WM_CLIPBOARDUPDATE event from Windows");

            // Read the newly copied text under the shared process lock
            let text_opt = {
                let _lock = sync_lock.lock().unwrap();
                read_clipboard_text_direct().unwrap_or(None)
            };

            if let Some(ClipboardContent::Text(text)) = text_opt {
                debug!(
                    len = text.len(),
                    "Broadcasting clipboard update from Windows event"
                );
                let _ = tx.send(ClipboardContent::Text(text));
            }
        } else if msg.message == WM_CLOSE {
            break;
        }

        // SAFETY: Translating and dispatching window message
        unsafe {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }

    // Clean up listener registration and window
    // SAFETY: Cleaning up Win32 resources
    unsafe {
        RemoveClipboardFormatListener(hwnd);
        DestroyWindow(hwnd);
        UnregisterClassW(class_name.as_ptr(), std::ptr::null_mut());
    }

    hwnd_out.store(0, Ordering::Release);
    debug!("Win32 clipboard listener loop exited cleanly");
}

/// Window procedure callback for the listener window.
unsafe extern "system" fn listener_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_DESTROY {
        // Post quit message to end GetMessageW loop
        windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
        0
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// Internal helper to read clipboard text directly.
fn read_clipboard_text_direct() -> Result<Option<ClipboardContent>, ClipboardError> {
    // SAFETY: Check format availability
    let available = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) };
    if available == 0 {
        return Ok(None);
    }

    let _guard = open_clipboard_with_retry()?;

    // SAFETY: GetClipboardData
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return Ok(None);
    }

    // SAFETY: Lock handle
    let ptr = unsafe { GlobalLock(handle) } as *const u16;
    if ptr.is_null() {
        return Ok(None);
    }

    let byte_size = unsafe { GlobalSize(handle) };
    let max_elements = byte_size / 2;

    let mut len = 0;
    while len < max_elements {
        let ch = unsafe { *ptr.add(len) };
        if ch == 0 {
            break;
        }
        len += 1;
    }

    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let text = String::from_utf16_lossy(slice);

    unsafe {
        GlobalUnlock(handle);
    }

    Ok(Some(ClipboardContent::Text(text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_windows_clipboard_set_get_unicode() {
        let backend = WindowsClipboardBackend::new().expect("Failed to initialize Windows backend");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let unique_payload = format!("BridgeOS Unit Test Payload: {nanos}");

        backend
            .set_content(ClipboardContent::Text(unique_payload.clone()))
            .expect("set_content failed");

        let retrieved = backend.get_content().expect("get_content failed");
        assert_eq!(
            retrieved,
            Some(ClipboardContent::Text(unique_payload)),
            "Retrieved clipboard content must match written payload"
        );
    }

    #[test]
    fn test_windows_clipboard_event_notification() {
        let backend = WindowsClipboardBackend::new().expect("Failed to initialize Windows backend");
        let mut rx = backend.subscribe();

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let unique_payload = format!("BridgeOS Event Notification: {nanos}");

        backend
            .set_content(ClipboardContent::Text(unique_payload.clone()))
            .expect("set_content failed");

        // Give the Windows message loop a moment to receive WM_CLIPBOARDUPDATE and broadcast
        let received = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            rt.block_on(async {
                tokio::time::timeout(Duration::from_millis(2000), rx.recv()).await
            })
        })
        .join()
        .unwrap();

        match received {
            Ok(Ok(ClipboardContent::Text(t))) => {
                assert_eq!(t, unique_payload, "Event listener received text must match");
            }
            other => panic!("Expected clipboard text event, received: {other:?}"),
        }
    }

    #[test]
    fn test_windows_clipboard_clean_shutdown() {
        let backend = WindowsClipboardBackend::new().expect("Failed to initialize Windows backend");
        drop(backend);
    }
}
