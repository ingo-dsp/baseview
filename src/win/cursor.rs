use winapi::{
    shared::ntdef::PCWSTR,
    um::winuser::{
        IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_WAIT, IDC_SIZENS, IDC_SIZEWE, IDC_SIZENESW, IDC_SIZENWSE,
    },
};

use crate::MouseCursor;

impl MouseCursor {
    pub(crate) fn to_windows_cursor(self) -> PCWSTR {
        match self {
            MouseCursor::Default => IDC_ARROW,
            MouseCursor::Hand | MouseCursor::Pointer => IDC_HAND,
            MouseCursor::HandGrabbing
            | MouseCursor::Move
            | MouseCursor::ZoomIn
            | MouseCursor::ZoomOut
            | MouseCursor::AllScroll => IDC_SIZEALL,
            MouseCursor::Help => IDC_HELP,
            MouseCursor::Text | MouseCursor::VerticalText => IDC_IBEAM,
            MouseCursor::Working | MouseCursor::PtrWorking => IDC_WAIT,
            MouseCursor::NotAllowed | MouseCursor::PtrNotAllowed => IDC_NO,
            MouseCursor::Crosshair => IDC_CROSS,
            MouseCursor::EResize | MouseCursor::WResize | MouseCursor::EwResize | MouseCursor::ColResize => IDC_SIZEWE,
            MouseCursor::NResize | MouseCursor::SResize | MouseCursor::NsResize | MouseCursor::RowResize => IDC_SIZENS,
            MouseCursor::NeResize | MouseCursor::SwResize | MouseCursor::NeswResize => IDC_SIZENESW,
            MouseCursor::NwResize | MouseCursor::SeResize | MouseCursor::NwseResize => IDC_SIZENWSE,
            _ => IDC_ARROW,
        }
    }
}
