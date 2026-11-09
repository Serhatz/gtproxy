//! Embedded mode: the Growtopia window sits over the free area of ours.
//!
//! Plain Win32: find the game's top-level window (process `Growtopia.exe`),
//! take its title bar off, make it an *owned* window of ours (it stays above
//! our window, not above other apps, and minimises with it) and keep it
//! centred over the free area. Nothing is injected into the game.
//!
//! Why not a child window (`SetParent`), tried first: the game is DPI-unaware
//! and does not follow a new window size. As a child, Windows stops scaling it
//! and the picture came out cropped whatever size it got (seen in game three
//! times). As an owned top-level window it is drawn exactly as it normally is;
//! its size is the resolution picked in the game's options (stretched by the
//! Windows scale factor), so it is only moved, never resized.
//!
//! When embedded mode is turned off or the proxy closes, the window gets its
//! old style, owner and position back.

use std::time::{Duration, Instant};

/// How often to look for the game while it is not embedded.
const SEARCH_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not on Windows, or embedded mode is off.
    Off,
    /// Waiting for the game window.
    Searching,
    Embedded,
}

#[derive(Default)]
pub struct Embed {
    #[cfg(windows)]
    game: Option<win::Attached>,
    last_search: Option<Instant>,
}

impl Embed {
    /// Keeps the game over `area` (`x, y, w, h` in physical pixels of the
    /// owner's client area), centred. Call every frame while embedded mode is on.
    #[allow(unused_variables)]
    pub fn update(&mut self, owner: isize, area: [i32; 4]) -> Status {
        #[cfg(windows)]
        {
            if let Some(g) = &mut self.game {
                if !g.alive() {
                    tracing::info!("embedded game window is gone");
                    self.game = None;
                } else {
                    g.keep(owner, area);
                    return Status::Embedded;
                }
            }
            let due = self.last_search.is_none_or(|t| t.elapsed() >= SEARCH_EVERY);
            if due {
                self.last_search = Some(Instant::now());
                if let Some(hwnd) = win::find_game() {
                    tracing::info!("embedding Growtopia window {hwnd:#x}");
                    let mut g = win::Attached::attach(hwnd, owner);
                    g.keep(owner, area);
                    self.game = Some(g);
                    return Status::Embedded;
                }
            }
            Status::Searching
        }
        #[cfg(not(windows))]
        Status::Off
    }

    /// Gives the game window back as a normal window.
    pub fn release(&mut self) {
        #[cfg(windows)]
        if let Some(g) = self.game.take() {
            tracing::info!("releasing the Growtopia window");
            g.release();
        }
        self.last_search = None;
    }

    /// The game's size on screen in pixels, once embedded.
    pub fn game_size(&self) -> Option<(i32, i32)> {
        #[cfg(windows)]
        return self.game.as_ref().map(|g| g.size);
        #[cfg(not(windows))]
        None
    }

    /// The game window, once embedded (the side panel is owned by it, so it
    /// always stays above the game).
    pub fn game_hwnd(&self) -> Option<isize> {
        #[cfg(windows)]
        return self.game.as_ref().map(|g| g.hwnd);
        #[cfg(not(windows))]
        None
    }

    /// Gives the game the keyboard (after a click on our panels it stays with us).
    pub fn focus_game(&self) {
        #[cfg(windows)]
        if let Some(g) = &self.game {
            g.focus();
        }
    }
}

/// Makes our window titled `title` an owned window of `owner`: it then stays
/// above the owner without being topmost for every app, and minimises with
/// it. `true` once done.
#[allow(unused_variables)]
pub fn own_window(title: &str, owner: isize) -> bool {
    #[cfg(windows)]
    return win::own_window(title, owner);
    #[cfg(not(windows))]
    false
}

impl Drop for Embed {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(windows)]
mod win {
    #![allow(non_snake_case, clippy::upper_case_acronyms)]

    use std::ffi::c_void;

    type HWND = isize;
    type BOOL = i32;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct POINT {
        x: i32,
        y: i32,
    }

    const GWL_STYLE: i32 = -16;
    const GWL_EXSTYLE: i32 = -20;
    const GWLP_HWNDPARENT: i32 = -8;
    const WS_CAPTION: isize = 0x00C0_0000;
    const WS_THICKFRAME: isize = 0x0004_0000;
    const WS_SYSMENU: isize = 0x0008_0000;
    const WS_MINIMIZEBOX: isize = 0x0002_0000;
    const WS_MAXIMIZEBOX: isize = 0x0001_0000;
    const WS_EX_APPWINDOW: isize = 0x0004_0000;
    const WS_EX_WINDOWEDGE: isize = 0x0000_0100;
    const WS_EX_CLIENTEDGE: isize = 0x0000_0200;
    const WS_EX_DLGMODALFRAME: isize = 0x0000_0001;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SW_RESTORE: i32 = 9;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(cb: extern "system" fn(HWND, isize) -> BOOL, lparam: isize) -> BOOL;
        fn IsWindowVisible(h: HWND) -> BOOL;
        fn IsWindow(h: HWND) -> BOOL;
        fn IsIconic(h: HWND) -> BOOL;
        fn IsZoomed(h: HWND) -> BOOL;
        fn ShowWindow(h: HWND, cmd: i32) -> BOOL;
        fn GetWindowThreadProcessId(h: HWND, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(h: HWND, index: i32) -> isize;
        fn SetWindowLongPtrW(h: HWND, index: i32, value: isize) -> isize;
        fn GetWindowRect(h: HWND, r: *mut RECT) -> BOOL;
        fn GetClientRect(h: HWND, r: *mut RECT) -> BOOL;
        fn ClientToScreen(h: HWND, p: *mut POINT) -> BOOL;
        fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, w: i32, h2: i32, flags: u32) -> BOOL;
        fn SetForegroundWindow(h: HWND) -> BOOL;
        fn FindWindowW(class: *const u16, title: *const u16) -> HWND;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> *mut c_void;
        fn CloseHandle(h: *mut c_void) -> BOOL;
        fn QueryFullProcessImageNameW(h: *mut c_void, flags: u32, name: *mut u16, size: *mut u32) -> BOOL;
        fn GetCurrentProcessId() -> u32;
    }

    pub fn own_window(title: &str, owner: HWND) -> bool {
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let h = unsafe { FindWindowW(std::ptr::null(), wide.as_ptr()) };
        if h == 0 {
            return false;
        }
        let mut pid = 0u32;
        // SAFETY: plain Win32 calls; only our own process's window is changed.
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            if pid != GetCurrentProcessId() {
                return false;
            }
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, owner);
        }
        true
    }

    /// Client size in screen pixels (our process is DPI aware).
    fn client_size(h: HWND) -> (i32, i32) {
        let mut r = RECT::default();
        // SAFETY: plain Win32 call with a valid out pointer.
        unsafe { GetClientRect(h, &mut r) };
        (r.right - r.left, r.bottom - r.top)
    }

    /// File name of the process that owns `h`, lowercase.
    fn exe_of(h: HWND) -> Option<String> {
        let mut pid = 0u32;
        // SAFETY: plain Win32 calls with valid out pointers; the handle is closed below.
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            if pid == 0 || pid == GetCurrentProcessId() {
                return None;
            }
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if p.is_null() {
                return None;
            }
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(p, 0, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(p);
            if !ok {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().map(str::to_lowercase)
        }
    }

    extern "system" fn each_window(h: HWND, out: isize) -> BOOL {
        // SAFETY: `out` is the `&mut Option<HWND>` passed to EnumWindows below.
        let found = unsafe { &mut *(out as *mut Option<HWND>) };
        // SAFETY: `h` comes from EnumWindows.
        let visible = unsafe { IsWindowVisible(h) } != 0;
        if visible && exe_of(h).as_deref() == Some("growtopia.exe") {
            *found = Some(h);
            return 0; // stop
        }
        1
    }

    /// The visible top-level window of `Growtopia.exe`.
    pub fn find_game() -> Option<HWND> {
        let mut found: Option<HWND> = None;
        // SAFETY: the callback only writes into `found`, which outlives the call.
        unsafe { EnumWindows(each_window, &mut found as *mut _ as isize) };
        found
    }

    pub struct Attached {
        pub hwnd: HWND,
        style: isize,
        exstyle: isize,
        owner: isize,
        rect: RECT,
        /// Client size on screen; the game draws this and we never change it.
        pub size: (i32, i32),
        placed: Option<(i32, i32)>,
    }

    impl Attached {
        pub fn attach(hwnd: HWND, owner: HWND) -> Self {
            let mut rect = RECT::default();
            // SAFETY: Win32 calls on window handles; failures leave the window as it was.
            unsafe {
                if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                GetWindowRect(hwnd, &mut rect);
                let me = Self {
                    hwnd,
                    style: GetWindowLongPtrW(hwnd, GWL_STYLE),
                    exstyle: GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                    owner: GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT),
                    rect,
                    size: client_size(hwnd),
                    placed: None,
                };
                tracing::info!("game picture {}x{} screen pixels", me.size.0, me.size.1);
                me.take(owner);
                me
            }
        }

        /// No title bar or border, owned by us. The window keeps its client
        /// size, so the game keeps drawing the same picture.
        unsafe fn take(&self, owner: HWND) {
            // SAFETY: see `attach`.
            unsafe {
                let frame = WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, self.style & !frame);
                let ex = WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME;
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, self.exstyle & !ex);
                SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, owner);
                let (w, h) = self.size;
                SetWindowPos(self.hwnd, 0, self.rect.left, self.rect.top, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            }
        }

        pub fn alive(&self) -> bool {
            // SAFETY: IsWindow accepts any value.
            unsafe { IsWindow(self.hwnd) != 0 }
        }

        /// Moves the game to the middle of `area` (owner client pixels). Also
        /// re-takes it if the game rebuilt its window style (resolution change,
        /// full screen), and follows a new size.
        pub fn keep(&mut self, owner: HWND, area: [i32; 4]) {
            // SAFETY: Win32 calls on window handles.
            unsafe {
                let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
                if GetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT) != owner || style & WS_CAPTION == WS_CAPTION {
                    self.size = client_size(self.hwnd);
                    self.take(owner);
                    self.placed = None;
                }
            }
            let now = client_size(self.hwnd);
            if now != self.size && now.0 > 0 && now.1 > 0 {
                tracing::info!("game picture now {}x{}", now.0, now.1);
                self.size = now;
                self.placed = None;
            }
            let mut origin = POINT { x: area[0], y: area[1] };
            // SAFETY: valid out pointer.
            unsafe { ClientToScreen(owner, &mut origin) };
            let (w, h) = self.size;
            let spot = (origin.x + ((area[2] - w) / 2).max(0), origin.y + ((area[3] - h) / 2).max(0));
            if self.placed != Some(spot) {
                // SAFETY: Win32 call on the game window; the size stays as it is.
                unsafe { SetWindowPos(self.hwnd, 0, spot.0, spot.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
                self.placed = Some(spot);
            }
        }

        pub fn focus(&self) {
            // SAFETY: we are the foreground app when this runs (a click on our bar).
            unsafe { SetForegroundWindow(self.hwnd) };
        }

        pub fn release(self) {
            if !self.alive() {
                return;
            }
            // SAFETY: restores what `attach` read.
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, self.owner);
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, self.style);
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, self.exstyle);
                let r = self.rect;
                SetWindowPos(
                    self.hwnd,
                    0,
                    r.left,
                    r.top,
                    (r.right - r.left).max(320),
                    (r.bottom - r.top).max(240),
                    SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                );
            }
        }
    }
}
