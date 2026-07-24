#![allow(unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;

use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BLACK_BRUSH, BeginPaint, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawTextW, EndPaint,
            FillRect, GetStockObject, HBRUSH, InvalidateRect, PAINTSTRUCT, SetBkMode, SetTextColor,
            TRANSPARENT,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
            GetSystemMetrics, HWND_TOPMOST, KillTimer, LWA_ALPHA, PostQuitMessage, RegisterClassW,
            SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA, SWP_HIDEWINDOW, SWP_NOACTIVATE,
            SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
            SystemParametersInfoW, TranslateMessage, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
    core::PCWSTR,
};

const CLASS_NAME: &str = "monitorctl-osd";
const TIMER: usize = 1;
const WIDTH: i32 = 560;
const HEIGHT: i32 = 72;

struct State {
    window: HWND,
    text: Vec<u16>,
    quit_on_hide: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        window: HWND::default(),
        text: Vec::new(),
        quit_on_hide: false,
    });
}

pub unsafe fn initialize(quit_on_hide: bool) -> Result<(), String> {
    let class_name = wide(CLASS_NAME);
    RegisterClassW(&WNDCLASSW {
        lpszClassName: PCWSTR(class_name.as_ptr()),
        lpfnWndProc: Some(window_proc),
        ..Default::default()
    });
    let window = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
        PCWSTR(class_name.as_ptr()),
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .map_err(|error| format!("cannot create OSD window: {error}"))?;
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.window = window;
        state.quit_on_hide = quit_on_hide;
    });
    Ok(())
}

pub unsafe fn show(message: &str, duration: u32, opacity: f32) -> Result<(), String> {
    validate_opacity(opacity)?;
    let window = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.text = message.encode_utf16().collect();
        state.window
    });
    if window.0.is_null() {
        return Err("OSD is not initialized".into());
    }
    let alpha = (opacity * 255.0).round() as u8;
    SetLayeredWindowAttributes(window, COLORREF(0), alpha, LWA_ALPHA)
        .map_err(|error| format!("cannot set OSD opacity: {error}"))?;
    let mut area = RECT {
        right: GetSystemMetrics(SM_CXSCREEN),
        bottom: GetSystemMetrics(SM_CYSCREEN),
        ..Default::default()
    };
    let _ = SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        Some(std::ptr::from_mut(&mut area).cast()),
        Default::default(),
    );
    let x = area.left + (area.right - area.left - WIDTH) / 2;
    let y = area.bottom - HEIGHT - 32;
    let _ = SetWindowPos(
        window,
        Some(HWND_TOPMOST),
        x,
        y,
        WIDTH,
        HEIGHT,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    let _ = InvalidateRect(Some(window), None, true);
    SetTimer(Some(window), TIMER, duration, None);
    Ok(())
}

pub unsafe fn show_blocking(message: &str, duration: u32, opacity: f32) -> Result<(), String> {
    initialize(true)?;
    show(message, duration, opacity)?;
    let mut message = Default::default();
    while GetMessageW(&mut message, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    Ok(())
}

pub fn validate_opacity(opacity: f32) -> Result<(), String> {
    (opacity.is_finite() && (0.10..=1.0).contains(&opacity))
        .then_some(())
        .ok_or_else(|| "OSD opacity must be between 0.10 and 1.00".into())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => paint(window),
        WM_TIMER if wparam.0 == TIMER => {
            let _ = KillTimer(Some(window), TIMER);
            let _ = SetWindowPos(window, None, 0, 0, 0, 0, SWP_HIDEWINDOW);
            if STATE.with(|state| state.borrow().quit_on_hide) {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn paint(window: HWND) -> LRESULT {
    let mut paint = PAINTSTRUCT::default();
    let dc = BeginPaint(window, &mut paint);
    let mut rect = Default::default();
    let _ = GetClientRect(window, &mut rect);
    FillRect(dc, &rect, HBRUSH(GetStockObject(BLACK_BRUSH).0));
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, COLORREF(0x00ff_ffff));
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let _ = DrawTextW(
            dc,
            &mut state.text,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    });
    let _ = EndPaint(window, &paint);
    LRESULT(0)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::validate_opacity;

    #[test]
    fn validates_opacity_range() {
        assert!(validate_opacity(0.10).is_ok());
        assert!(validate_opacity(1.0).is_ok());
        assert!(validate_opacity(0.09).is_err());
        assert!(validate_opacity(1.01).is_err());
    }
}
