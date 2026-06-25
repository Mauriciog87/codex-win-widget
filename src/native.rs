use crate::app_server::CodexAppServerClient;
use crate::model::{StatusLevel, WidgetSnapshot};
use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    COLORREF, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
    RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET,
    DEFAULT_QUALITY, DRAW_TEXT_FORMAT, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE, FW_BOLD, FW_NORMAL, FillRect,
    HBRUSH, HDC, HFONT, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, RoundRect,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{
    NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    Shell_NotifyIconGetRect, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CalculatePopupWindowPosition, CreateIcon,
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW, HMENU, IDC_ARROW, LoadCursorW,
    MF_CHECKED, MF_STRING, MF_UNCHECKED, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SW_HIDE, SW_SHOW, SW_SHOWNA, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, ShowWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TPM_WORKAREA,
    TRACK_POPUP_MENU_FLAGS, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_ACTIVATE, WM_APP,
    WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_CREATE, WM_DESTROY, WM_DISPLAYCHANGE, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE,
    WM_PAINT, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_BORDER, WS_OVERLAPPEDWINDOW, WS_POPUP,
};
use windows::core::{GUID, PCWSTR, Result, w};

const WM_TRAY: u32 = WM_APP + 1;
const WM_SNAPSHOT: u32 = WM_APP + 2;
const TIMER_REFRESH: usize = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;
const VK_ESCAPE: usize = 0x1b;
const MENU_OPEN_PANEL: u16 = 1001;
const MENU_REFRESH: u16 = 1002;
const MENU_OPEN_CODEX: u16 = 1003;
const MENU_COPY_STATUS: u16 = 1004;
const MENU_START_WITH_WINDOWS: u16 = 1005;
const MENU_EXIT: u16 = 1006;
const ICON_SIZE: i32 = 32;
const FLYOUT_WIDTH: i32 = 360;
const FLYOUT_HEIGHT: i32 = 216;
const REFRESH_CACHE_TTL: Duration = Duration::from_secs(60);
const STARTUP_VALUE_NAME: &str = "CodexWinWidget";
const STARTUP_RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const TRAY_GUID: GUID = GUID::from_u128(0x8f21bb2e_4b80_4cf6_b4d5_201f44b9f247);

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let module = unsafe { GetModuleHandleW(None)? };
    let instance = HINSTANCE(module.0);
    register_window_classes(instance)?;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("CodexWinWidgetMain"),
            w!("Codex Windows Widget"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?
    };

    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }

    let state = Box::new(AppState::new(hwnd)?);
    let state_ptr = Box::into_raw(state);

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
    }

    with_state(hwnd, |state| {
        state.update_tray_icon();
        state.request_refresh();
    });

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0).as_bool() } {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

fn register_window_classes(instance: HINSTANCE) -> Result<()> {
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
    let main_class = WNDCLASSW {
        hCursor: cursor,
        hInstance: instance,
        lpszClassName: w!("CodexWinWidgetMain"),
        lpfnWndProc: Some(main_wnd_proc),
        ..Default::default()
    };
    let flyout_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        hCursor: cursor,
        hInstance: instance,
        lpszClassName: w!("CodexWinWidgetFlyout"),
        lpfnWndProc: Some(flyout_wnd_proc),
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
        ..Default::default()
    };

    unsafe {
        RegisterClassW(&main_class);
        RegisterClassW(&flyout_class);
    }

    Ok(())
}

struct AppState {
    hwnd: HWND,
    flyout_hwnd: Option<HWND>,
    current_icon: Option<windows::Win32::UI::WindowsAndMessaging::HICON>,
    snapshot: WidgetSnapshot,
    client: CodexAppServerClient,
    refresh_inflight: bool,
    last_refresh_at: Option<Instant>,
    taskbar_created_message: u32,
}

impl AppState {
    fn new(hwnd: HWND) -> Result<Self> {
        let taskbar_created_message = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        let snapshot = WidgetSnapshot::error("Loading limits");
        let icon = create_status_icon(snapshot.status_level())?;
        let mut state = Self {
            hwnd,
            flyout_hwnd: None,
            current_icon: Some(icon),
            snapshot,
            client: CodexAppServerClient::new(),
            refresh_inflight: false,
            last_refresh_at: None,
            taskbar_created_message,
        };
        state.add_tray_icon()?;
        unsafe {
            SetTimer(Some(hwnd), TIMER_REFRESH, 60_000, None);
        }
        Ok(state)
    }

    fn add_tray_icon(&mut self) -> Result<()> {
        let mut data = self.notify_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID | NIF_SHOWTIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.current_icon.unwrap_or_default();
        write_fixed_wstr(&mut data.szTip, &self.snapshot.tooltip());
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
        if added {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
        Ok(())
    }

    fn update_tray_icon(&mut self) {
        let Ok(icon) = create_status_icon(self.snapshot.status_level()) else {
            return;
        };

        let old_icon = self.current_icon.replace(icon);
        let mut data = self.notify_data();
        data.uFlags = NIF_ICON | NIF_TIP | NIF_GUID | NIF_SHOWTIP;
        data.hIcon = icon;
        write_fixed_wstr(&mut data.szTip, &self.snapshot.tooltip());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
        if let Some(old_icon) = old_icon {
            unsafe {
                let _ = DestroyIcon(old_icon);
            }
        }
        self.invalidate_flyout();
    }

    fn remove_tray_icon(&mut self) {
        let data = self.notify_data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        if let Some(icon) = self.current_icon.take() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }

    fn notify_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            guidItem: TRAY_GUID,
            ..Default::default()
        }
    }

    fn request_refresh(&mut self) {
        self.request_refresh_with_cache(false);
    }

    fn request_refresh_now(&mut self) {
        self.request_refresh_with_cache(true);
    }

    fn request_refresh_with_cache(&mut self, force: bool) {
        if self.refresh_inflight {
            return;
        }
        if !force
            && let Some(last_refresh_at) = self.last_refresh_at
            && last_refresh_at.elapsed() < REFRESH_CACHE_TTL
        {
            return;
        }
        self.refresh_inflight = true;
        let hwnd_value = self.hwnd.0 as isize;
        let client = self.client.clone();
        thread::spawn(move || {
            let snapshot = client.fetch_snapshot();
            let boxed = Box::new(snapshot);
            let ptr = Box::into_raw(boxed);
            unsafe {
                if PostMessageW(
                    Some(HWND(hwnd_value as *mut c_void)),
                    WM_SNAPSHOT,
                    WPARAM(ptr as usize),
                    LPARAM(0),
                )
                .is_err()
                {
                    drop(Box::from_raw(ptr));
                }
            }
        });
    }

    fn apply_snapshot(&mut self, snapshot: WidgetSnapshot) {
        self.snapshot = snapshot;
        self.refresh_inflight = false;
        self.last_refresh_at = Some(Instant::now());
        self.update_tray_icon();
    }

    fn show_flyout(&mut self, anchor_rect: Option<RECT>) {
        self.request_refresh();
        let position = self.flyout_position(anchor_rect);

        let hwnd = if let Some(hwnd) = self.flyout_hwnd {
            hwnd
        } else {
            let Ok(module) = (unsafe { GetModuleHandleW(None) }) else {
                return;
            };
            let instance = HINSTANCE(module.0);
            let state_ptr = self as *mut AppState as *mut c_void;
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0x00000080),
                    w!("CodexWinWidgetFlyout"),
                    w!("Codex Limits"),
                    WS_POPUP | WS_BORDER,
                    position.left,
                    position.top,
                    FLYOUT_WIDTH,
                    FLYOUT_HEIGHT,
                    Some(self.hwnd),
                    None,
                    Some(instance),
                    Some(state_ptr),
                )
            };
            let Ok(hwnd) = hwnd else {
                return;
            };
            self.flyout_hwnd = Some(hwnd);
            hwnd
        };

        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
                position.left,
                position.top,
                FLYOUT_WIDTH,
                FLYOUT_HEIGHT,
                windows::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(hwnd);
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }

    fn flyout_position(&self, anchor_rect: Option<RECT>) -> RECT {
        let icon_rect = anchor_rect
            .or_else(|| self.tray_rect())
            .unwrap_or_else(cursor_rect);
        let anchor = POINT {
            x: icon_rect.left,
            y: icon_rect.top,
        };
        let size = SIZE {
            cx: FLYOUT_WIDTH,
            cy: FLYOUT_HEIGHT,
        };
        let mut output = RECT::default();
        let ok = unsafe {
            CalculatePopupWindowPosition(
                &anchor,
                &size,
                (TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_WORKAREA).0,
                Some(&icon_rect),
                &mut output,
            )
            .is_ok()
        };
        if ok {
            offset_rect(output, 0, -8)
        } else {
            RECT {
                left: icon_rect.left - FLYOUT_WIDTH,
                top: icon_rect.top - size.cy - 8,
                right: icon_rect.left,
                bottom: icon_rect.top - 8,
            }
        }
    }

    fn tray_rect(&self) -> Option<RECT> {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            guidItem: TRAY_GUID,
        };
        unsafe { Shell_NotifyIconGetRect(&identifier) }.ok()
    }

    fn show_context_menu(&mut self) {
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };
        unsafe {
            append_menu_item(menu, MENU_OPEN_PANEL, "Show limits");
            append_menu_item(menu, MENU_REFRESH, "Refresh now");
            append_menu_item(menu, MENU_OPEN_CODEX, "Open Codex");
            append_menu_item(menu, MENU_COPY_STATUS, "Copy status");
            append_checked_menu_item(
                menu,
                MENU_START_WITH_WINDOWS,
                "Start with Windows",
                startup_enabled(),
            );
            append_menu_item(menu, MENU_EXIT, "Quit");
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = TrackPopupMenu(
                menu,
                TRACK_POPUP_MENU_FLAGS(TPM_LEFTALIGN.0 | TPM_BOTTOMALIGN.0 | TPM_RIGHTBUTTON.0),
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
        }
    }

    fn handle_command(&mut self, id: u16) {
        match id {
            MENU_OPEN_PANEL => self.show_flyout(Some(cursor_rect())),
            MENU_REFRESH => self.request_refresh_now(),
            MENU_OPEN_CODEX => open_url("https://chatgpt.com/codex"),
            MENU_COPY_STATUS => copy_to_clipboard(self.hwnd, &self.snapshot.status_summary()),
            MENU_START_WITH_WINDOWS => {
                let _ = set_startup_enabled(!startup_enabled());
            }
            MENU_EXIT => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }

    fn invalidate_flyout(&self) {
        if let Some(hwnd) = self.flyout_hwnd {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        self.remove_tray_icon();
    }
}

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DESTROY {
        let ptr = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut AppState };
        if !ptr.is_null() {
            let _ = unsafe { Box::from_raw(ptr) };
        }
        unsafe {
            PostQuitMessage(0);
        }
        return LRESULT(0);
    }

    if let Some(Some(result)) = with_state(hwnd, |state| {
        if message == state.taskbar_created_message {
            let _ = state.add_tray_icon();
            state.update_tray_icon();
            return Some(LRESULT(0));
        }

        match message {
            WM_CREATE => Some(LRESULT(0)),
            WM_TIMER if wparam.0 == TIMER_REFRESH => {
                state.request_refresh();
                Some(LRESULT(0))
            }
            WM_SNAPSHOT => {
                let ptr = wparam.0 as *mut WidgetSnapshot;
                if !ptr.is_null() {
                    let snapshot = unsafe { *Box::from_raw(ptr) };
                    state.apply_snapshot(snapshot);
                }
                Some(LRESULT(0))
            }
            WM_TRAY => {
                if let Some(action) = tray_action_from_lparam(lparam) {
                    match action {
                        TrayAction::OpenFlyout => {
                            let anchor =
                                tray_anchor_from_wparam(wparam).or_else(|| Some(cursor_rect()));
                            state.show_flyout(anchor);
                        }
                        TrayAction::ShowContextMenu => state.show_context_menu(),
                    }
                }
                Some(LRESULT(0))
            }
            WM_COMMAND => {
                state.handle_command((wparam.0 & 0xffff) as u16);
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE => {
                state.update_tray_icon();
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }) {
        return result;
    }

    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn flyout_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create = lparam.0 as *const CREATESTRUCTW;
            if !create.is_null() {
                let ptr = unsafe { (*create).lpCreateParams };
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
                }
            }
            LRESULT(1)
        }
        WM_PAINT => {
            let ptr = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
                    as *mut AppState
            };
            if !ptr.is_null() {
                let state = unsafe { &*ptr };
                paint_flyout(hwnd, &state.snapshot);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            track_mouse_leave(hwnd);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_ACTIVATE
            if low_word(wparam.0 as isize)
                == windows::Win32::UI::WindowsAndMessaging::WA_INACTIVE =>
        {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn track_mouse_leave(hwnd: HWND) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut event);
    }
}

fn with_state<T>(hwnd: HWND, f: impl FnOnce(&mut AppState) -> T) -> Option<T> {
    let ptr = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
            as *mut AppState
    };
    if ptr.is_null() {
        None
    } else {
        Some(f(unsafe { &mut *ptr }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    OpenFlyout,
    ShowContextMenu,
}

fn tray_action_from_lparam(lparam: LPARAM) -> Option<TrayAction> {
    match low_word(lparam.0) {
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => {
            Some(TrayAction::OpenFlyout)
        }
        WM_RBUTTONUP | WM_CONTEXTMENU => Some(TrayAction::ShowContextMenu),
        _ => None,
    }
}

fn tray_anchor_from_wparam(wparam: WPARAM) -> Option<RECT> {
    let value = wparam.0 as u32;
    let x = (value & 0xffff) as i16 as i32;
    let y = ((value >> 16) & 0xffff) as i16 as i32;
    if (0..=4).contains(&x) && (0..=4).contains(&y) {
        return None;
    }
    Some(RECT {
        left: x,
        top: y,
        right: x + 1,
        bottom: y + 1,
    })
}

fn low_word(value: isize) -> u32 {
    (value as u32) & 0xffff
}

fn offset_rect(rect: RECT, dx: i32, dy: i32) -> RECT {
    RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right + dx,
        bottom: rect.bottom + dy,
    }
}

fn cursor_rect() -> RECT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    RECT {
        left: point.x,
        top: point.y,
        right: point.x + 1,
        bottom: point.y + 1,
    }
}

fn create_status_icon(
    level: StatusLevel,
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
    let accent = match level {
        StatusLevel::Good => [78, 210, 121, 255],
        StatusLevel::Warning => [245, 184, 73, 255],
        StatusLevel::Critical => [246, 103, 96, 255],
        StatusLevel::Unknown => [154, 163, 178, 255],
        StatusLevel::Error => [246, 103, 96, 255],
    };

    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[29, 31, 36, 255]);
    }

    for y in 28..ICON_SIZE {
        for x in 0..ICON_SIZE {
            fill_pixel_block(&mut rgba, x, y, 1, accent);
        }
    }
    draw_text_bitmap(&mut rgba, "C", 0, 2, 4, accent);

    let mut xor = vec![0u8; rgba.len()];
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let source = ((y * ICON_SIZE + x) * 4) as usize;
            let target = (((ICON_SIZE - 1 - y) * ICON_SIZE + x) * 4) as usize;
            xor[target] = rgba[source + 2];
            xor[target + 1] = rgba[source + 1];
            xor[target + 2] = rgba[source];
            xor[target + 3] = rgba[source + 3];
        }
    }
    let and_mask = vec![0u8; ((ICON_SIZE * ICON_SIZE) / 8) as usize];

    let module = unsafe { GetModuleHandleW(None)? };
    let instance = HINSTANCE(module.0);
    let icon = unsafe {
        CreateIcon(
            Some(instance),
            ICON_SIZE,
            ICON_SIZE,
            1,
            32,
            and_mask.as_ptr(),
            xor.as_ptr(),
        )?
    };
    Ok(icon)
}

fn draw_text_bitmap(
    buffer: &mut [u8],
    text: &str,
    center_x: i32,
    y: i32,
    scale: i32,
    color: [u8; 4],
) {
    let glyphs = text.chars().filter_map(glyph).collect::<Vec<_>>();
    if glyphs.is_empty() {
        return;
    }
    let width = glyphs.len() as i32 * 5 * scale + (glyphs.len() as i32 - 1) * scale;
    let start_x = center_x + ((ICON_SIZE - width) / 2).max(0);
    for (index, rows) in glyphs.iter().enumerate() {
        let glyph_x = start_x + index as i32 * 6 * scale;
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    fill_pixel_block(
                        buffer,
                        glyph_x + col * scale,
                        y + row as i32 * scale,
                        scale,
                        color,
                    );
                }
            }
        }
    }
}

fn fill_pixel_block(buffer: &mut [u8], x: i32, y: i32, size: i32, color: [u8; 4]) {
    for yy in y..(y + size) {
        for xx in x..(x + size) {
            if (0..ICON_SIZE).contains(&xx) && (0..ICON_SIZE).contains(&yy) {
                let index = ((yy * ICON_SIZE + xx) * 4) as usize;
                buffer[index..index + 4].copy_from_slice(&color);
            }
        }
    }
}

fn glyph(ch: char) -> Option<[u8; 7]> {
    match ch.to_ascii_uppercase() {
        'C' => Some([
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ]),
        _ => None,
    }
}

fn paint_flyout(hwnd: HWND, snapshot: &WidgetSnapshot) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc.is_invalid() {
        return;
    }

    let mut rect = RECT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
    }

    let view = snapshot.card_view();
    let bg = unsafe { CreateSolidBrush(rgb(31, 31, 31)) };

    unsafe {
        FillRect(hdc, &rect, bg);
        SetBkMode(hdc, TRANSPARENT);
    }

    let title_font = create_font(15, FW_BOLD.0 as i32);
    let subtitle_font = create_font(12, FW_NORMAL.0 as i32);
    let value_font = create_font(36, FW_BOLD.0 as i32);
    let weekly_font = create_font(24, FW_BOLD.0 as i32);
    let label_font = create_font(12, FW_NORMAL.0 as i32);

    let card = RECT {
        left: 10,
        top: 10,
        right: FLYOUT_WIDTH - 10,
        bottom: FLYOUT_HEIGHT - 10,
    };
    draw_round_rect(hdc, card, rgb(36, 36, 36), rgb(61, 61, 61), 12);
    draw_round_rect(
        hdc,
        RECT {
            left: 26,
            top: 30,
            right: 38,
            bottom: 42,
        },
        color_for_level(view.status_level),
        color_for_level(view.status_level),
        8,
    );

    draw_text(
        hdc,
        title_font,
        text_spec(&view.title, 46, 23, 270, 24, rgb(245, 245, 245), DT_LEFT),
    );
    draw_text(
        hdc,
        subtitle_font,
        text_spec(&view.subtitle, 46, 45, 270, 22, rgb(207, 207, 207), DT_LEFT),
    );
    let weekly_card = RECT {
        left: 190,
        top: 48,
        right: FLYOUT_WIDTH - 24,
        bottom: 116,
    };
    draw_round_rect(hdc, weekly_card, rgb(42, 42, 42), rgb(66, 66, 66), 8);
    draw_text(
        hdc,
        label_font,
        text_spec("Week", 206, 54, 120, 18, rgb(210, 210, 210), DT_LEFT),
    );
    draw_text(
        hdc,
        weekly_font,
        text_spec(
            &view.weekly_value,
            206,
            70,
            100,
            30,
            rgb(255, 255, 255),
            DT_LEFT,
        ),
    );
    draw_text(
        hdc,
        label_font,
        text_spec(
            &view.weekly_label,
            206,
            96,
            124,
            18,
            rgb(202, 202, 202),
            DT_LEFT,
        ),
    );
    draw_text(
        hdc,
        value_font,
        text_spec(
            &view.primary_value,
            26,
            68,
            160,
            50,
            rgb(255, 255, 255),
            DT_LEFT,
        ),
    );
    draw_text(
        hdc,
        label_font,
        text_spec(
            &view.primary_label,
            38,
            104,
            120,
            18,
            rgb(202, 202, 202),
            DT_LEFT,
        ),
    );
    draw_progress_bar(hdc, 26, 124, 308, view.progress_percent, view.status_level);
    draw_text(
        hdc,
        label_font,
        text_spec(
            &view.remaining_label,
            26,
            142,
            145,
            24,
            rgb(235, 235, 235),
            DT_LEFT,
        ),
    );
    draw_text(
        hdc,
        label_font,
        text_spec(
            &view.reset_label,
            176,
            142,
            150,
            24,
            rgb(235, 235, 235),
            DT_RIGHT,
        ),
    );
    if let Some(message) = &view.status_message {
        draw_text(
            hdc,
            label_font,
            text_spec(message, 26, 166, 300, 20, rgb(246, 160, 154), DT_LEFT),
        );
    } else if let Some(reset_credits) = &view.reset_credits_label {
        draw_text(
            hdc,
            label_font,
            text_spec(reset_credits, 26, 166, 300, 20, rgb(202, 202, 202), DT_LEFT),
        );
    }

    let usage = view
        .usage_lines
        .first()
        .map(|(_, value)| value.as_str())
        .unwrap_or("No usage yet");
    let footer = format!(
        "{}  |  Today {usage}  |  {}",
        view.credits_label, view.updated_label
    );
    draw_text(
        hdc,
        label_font,
        text_spec(&footer, 26, 186, 308, 20, rgb(190, 190, 190), DT_LEFT),
    );
    unsafe {
        let _ = DeleteObject(bg.into());
        let _ = DeleteObject(title_font.into());
        let _ = DeleteObject(subtitle_font.into());
        let _ = DeleteObject(value_font.into());
        let _ = DeleteObject(weekly_font.into());
        let _ = DeleteObject(label_font.into());
        let _ = EndPaint(hwnd, &ps);
    }
}

fn draw_progress_bar(hdc: HDC, x: i32, y: i32, width: i32, percent: f64, level: StatusLevel) {
    let track = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + 10,
    };
    draw_round_rect(hdc, track, rgb(75, 75, 75), rgb(75, 75, 75), 8);
    let fill_width = ((width as f64) * (percent.clamp(0.0, 100.0) / 100.0)).round() as i32;
    if fill_width > 0 {
        let fill = RECT {
            left: x,
            top: y,
            right: x + fill_width.max(10),
            bottom: y + 10,
        };
        draw_round_rect(hdc, fill, color_for_level(level), color_for_level(level), 8);
    }
}

fn draw_round_rect(hdc: HDC, rect: RECT, fill: COLORREF, border: COLORREF, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(pen.into());
        let _ = DeleteObject(brush.into());
    }
}

fn color_for_level(level: StatusLevel) -> COLORREF {
    match level {
        StatusLevel::Good => rgb(65, 181, 112),
        StatusLevel::Warning => rgb(230, 173, 63),
        StatusLevel::Critical => rgb(236, 91, 86),
        StatusLevel::Unknown => rgb(143, 150, 163),
        StatusLevel::Error => rgb(236, 91, 86),
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}

fn create_font(height: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            FF_DONTCARE.0 as u32,
            w!("Segoe UI"),
        )
    }
}

struct TextSpec<'a> {
    text: &'a str,
    rect: RECT,
    color: COLORREF,
    align: DRAW_TEXT_FORMAT,
}

fn text_spec(
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: COLORREF,
    align: DRAW_TEXT_FORMAT,
) -> TextSpec<'_> {
    TextSpec {
        text,
        rect: RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        },
        color,
        align,
    }
}

fn draw_text(hdc: HDC, font: HFONT, spec: TextSpec<'_>) {
    let mut rect = spec.rect;
    let mut wide = wide_null(spec.text);
    let text_len = wide.len().saturating_sub(1);
    unsafe {
        let old = SelectObject(hdc, font.into());
        SetTextColor(hdc, spec.color);
        SetBkMode(hdc, TRANSPARENT);
        DrawTextW(
            hdc,
            &mut wide[..text_len],
            &mut rect,
            spec.align | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        SelectObject(hdc, old);
    }
}

unsafe fn append_menu_item(menu: HMENU, id: u16, text: &str) {
    let wide = wide_null(text);
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(wide.as_ptr()));
    }
}

unsafe fn append_checked_menu_item(menu: HMENU, id: u16, text: &str, checked: bool) {
    let wide = wide_null(text);
    let checked_flag = if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        let _ = AppendMenuW(
            menu,
            MF_STRING | checked_flag,
            id as usize,
            PCWSTR(wide.as_ptr()),
        );
    }
}

fn open_url(url: &str) {
    let url = wide_null(url);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            None,
            None,
            SW_SHOWNA,
        );
    }
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn startup_enabled() -> bool {
    let Some(key) = open_startup_key(KEY_QUERY_VALUE) else {
        return false;
    };
    let name = wide_null(STARTUP_VALUE_NAME);
    let result = unsafe { RegQueryValueExW(key.0, PCWSTR(name.as_ptr()), None, None, None, None) };
    result == ERROR_SUCCESS
}

fn set_startup_enabled(enabled: bool) -> bool {
    if enabled {
        enable_startup()
    } else {
        disable_startup()
    }
}

fn enable_startup() -> bool {
    let Some(command) = startup_command() else {
        return false;
    };
    if command.encode_utf16().count() > 260 {
        return false;
    }
    let Some(key) = create_startup_key() else {
        return false;
    };
    let name = wide_null(STARTUP_VALUE_NAME);
    let command = wide_null(&command);
    let bytes = wide_bytes(&command);
    let result = unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) };
    result == ERROR_SUCCESS
}

fn disable_startup() -> bool {
    let Some(key) = open_startup_key(KEY_SET_VALUE) else {
        return true;
    };
    let name = wide_null(STARTUP_VALUE_NAME);
    let result = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND
}

fn open_startup_key(
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Option<RegistryKey> {
    let mut key = HKEY::default();
    let subkey = wide_null(STARTUP_RUN_KEY);
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            access,
            &mut key,
        )
    };
    (result == ERROR_SUCCESS).then_some(RegistryKey(key))
}

fn create_startup_key() -> Option<RegistryKey> {
    let mut key = HKEY::default();
    let subkey = wide_null(STARTUP_RUN_KEY);
    let result = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), &mut key) };
    (result == ERROR_SUCCESS).then_some(RegistryKey(key))
}

fn startup_command() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|path| format_startup_command(&path))
}

fn format_startup_command(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn wide_bytes(value: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(value.as_ptr() as *const u8, std::mem::size_of_val(value)) }
}

fn copy_to_clipboard(hwnd: HWND, text: &str) {
    let wide = wide_null(text);
    let bytes = wide.len() * size_of::<u16>();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes) {
            let ptr = GlobalLock(handle) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                let _ = GlobalUnlock(handle);
                let _ = SetClipboardData(13, Some(HANDLE(handle.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

fn write_fixed_wstr(buffer: &mut [u16], text: &str) {
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let limit = buffer.len().saturating_sub(1).min(wide.len());
    buffer[..limit].copy_from_slice(&wide[..limit]);
    if !buffer.is_empty() {
        buffer[limit] = 0;
    }
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_legacy_tray_mouse_messages() {
        assert_eq!(
            tray_action_from_lparam(LPARAM(WM_LBUTTONUP as isize)),
            Some(TrayAction::OpenFlyout)
        );
        assert_eq!(
            tray_action_from_lparam(LPARAM(WM_LBUTTONDOWN as isize)),
            Some(TrayAction::OpenFlyout)
        );
        assert_eq!(
            tray_action_from_lparam(LPARAM(WM_LBUTTONDBLCLK as isize)),
            Some(TrayAction::OpenFlyout)
        );
        assert_eq!(
            tray_action_from_lparam(LPARAM(WM_RBUTTONUP as isize)),
            Some(TrayAction::ShowContextMenu)
        );
    }

    #[test]
    fn maps_notify_icon_v4_tray_messages() {
        assert_eq!(
            tray_action_from_lparam(LPARAM(NIN_SELECT as isize)),
            Some(TrayAction::OpenFlyout)
        );
        assert_eq!(
            tray_action_from_lparam(LPARAM(NIN_KEYSELECT as isize)),
            Some(TrayAction::OpenFlyout)
        );
        assert_eq!(
            tray_action_from_lparam(LPARAM(WM_CONTEXTMENU as isize)),
            Some(TrayAction::ShowContextMenu)
        );
    }

    #[test]
    fn ignores_unhandled_tray_messages_even_with_icon_id_high_word() {
        let lparam = LPARAM(((1u32 << 16) | WM_TIMER) as isize);
        assert_eq!(tray_action_from_lparam(lparam), None);
    }

    #[test]
    fn extracts_notify_icon_anchor_from_wparam() {
        let anchor = tray_anchor_from_wparam(WPARAM(((650u32) << 16 | 1200u32) as usize));
        assert_eq!(
            anchor,
            Some(RECT {
                left: 1200,
                top: 650,
                right: 1201,
                bottom: 651
            })
        );
        assert_eq!(tray_anchor_from_wparam(WPARAM(1)), None);
    }

    #[test]
    fn quotes_startup_command_path() {
        assert_eq!(
            format_startup_command(Path::new(
                r"C:\Program Files\Codex Widget\codex-win-widget.exe"
            )),
            r#""C:\Program Files\Codex Widget\codex-win-widget.exe""#
        );
    }
}
