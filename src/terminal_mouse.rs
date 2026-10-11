//! Wheel routing follows the application's live mouse reporting mode. Ordinary
//! shell output stays in terminal scrollback; TUIs receive mouse reports instead.

use lgui::core::KeyModifiers;
use vt100::{MouseProtocolEncoding, MouseProtocolMode, Screen};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelTarget {
    History(bool),
    Application(MouseProtocolMode, MouseProtocolEncoding),
}

#[derive(Default)]
pub(crate) struct TerminalMouse {
    position: (u16, u16),
    pending: f32,
    target: Option<WheelTarget>,
}

impl TerminalMouse {
    pub fn set_position(&mut self, row: u16, col: u16) {
        self.position = (row, col);
    }

    /// Positive deltas scroll up. One wheel notch is one application event;
    /// applications apply their own speed, while local history moves three rows.
    pub fn wheel(&mut self, screen: &mut Screen, delta: f32, modifiers: KeyModifiers) -> Vec<u8> {
        if !delta.is_finite() || delta == 0.0 {
            return Vec::new();
        }
        let mode = screen.mouse_protocol_mode();
        let target = if modifiers.shift()
            || matches!(mode, MouseProtocolMode::None | MouseProtocolMode::Press)
        {
            // X10 (mode 9) reports button presses only, without wheel events.
            WheelTarget::History(screen.alternate_screen())
        } else {
            WheelTarget::Application(mode, screen.mouse_protocol_encoding())
        };
        if self.target != Some(target) || self.pending.signum() != delta.signum() {
            self.pending = 0.0;
            self.target = Some(target);
        }
        let scale = if matches!(target, WheelTarget::History(_)) {
            3.0
        } else {
            1.0
        };
        self.pending = (self.pending + delta * scale).clamp(-2_000.0, 2_000.0);
        let nearest = self.pending.round();
        if (self.pending - nearest).abs() < 0.000_001 {
            self.pending = nearest;
        }
        let count = self.pending.trunc() as i32;
        self.pending -= count as f32;
        if count == 0 {
            return Vec::new();
        }
        match target {
            WheelTarget::History(_) => {
                scroll_history(screen, count);
                Vec::new()
            }
            WheelTarget::Application(_, encoding) => {
                let (rows, cols) = screen.size();
                let row = self.position.0.min(rows.saturating_sub(1));
                let col = self.position.1.min(cols.saturating_sub(1));
                let button = if count > 0 { 64 } else { 65 }
                    + 8 * u8::from(modifiers.alt())
                    + 16 * u8::from(modifiers.ctrl());
                wheel_report(encoding, button, row, col).repeat(count.unsigned_abs() as usize)
            }
        }
    }
}

pub(crate) fn scroll_history(screen: &mut Screen, rows: i32) {
    let current = screen.scrollback();
    let next = if rows >= 0 {
        current.saturating_add(rows as usize)
    } else {
        current.saturating_sub(rows.unsigned_abs() as usize)
    };
    screen.set_scrollback(next);
}

fn wheel_report(encoding: MouseProtocolEncoding, button: u8, row: u16, col: u16) -> Vec<u8> {
    // Protocol coordinates are one-based. Do not wrap or misreport a position
    // outside the range of the older encodings.
    let x = u32::from(col) + 1;
    let y = u32::from(row) + 1;
    match encoding {
        MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{x};{y}M").into_bytes(),
        MouseProtocolEncoding::Default if x <= 223 && y <= 223 => {
            vec![
                0x1b,
                b'[',
                b'M',
                button + 32,
                (x + 32) as u8,
                (y + 32) as u8,
            ]
        }
        MouseProtocolEncoding::Utf8 if x <= 2015 && y <= 2015 => {
            let mut report = String::from("\x1b[M");
            for value in [u32::from(button) + 32, x + 32, y + 32] {
                report.push(char::from_u32(value).expect("mouse coordinate fits UTF-8"));
            }
            report.into_bytes()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_wheel_reports_match_live_tracking_modes() {
        for mode in [1000, 1002, 1003] {
            let mut parser = vt100::Parser::new(32, 120, 2000);
            parser.process(format!("\x1b[?1049h\x1b[?{mode}h\x1b[?1006h").as_bytes());
            let mut mouse = TerminalMouse::default();
            mouse.set_position(7, 13);
            assert_eq!(
                mouse.wheel(parser.screen_mut(), 1.0, KeyModifiers::empty()),
                b"\x1b[<64;14;8M"
            );
            assert_eq!(
                mouse.wheel(parser.screen_mut(), -2.0, KeyModifiers::empty()),
                b"\x1b[<65;14;8M\x1b[<65;14;8M"
            );
            assert_eq!(parser.screen().scrollback(), 0);
            assert_eq!(
                mouse.wheel(
                    parser.screen_mut(),
                    1.0,
                    KeyModifiers::CONTROL | KeyModifiers::ALT
                ),
                b"\x1b[<88;14;8M"
            );
        }
    }

    #[test]
    fn mouse_encodings_preserve_coordinates_without_overflow() {
        assert_eq!(
            wheel_report(MouseProtocolEncoding::Default, 64, 7, 13),
            b"\x1b[M`.("
        );
        assert_eq!(
            wheel_report(MouseProtocolEncoding::Default, 65, 222, 222),
            [0x1b, b'[', b'M', 97, 255, 255]
        );
        assert!(wheel_report(MouseProtocolEncoding::Default, 64, 0, 223).is_empty());
        assert_eq!(
            wheel_report(MouseProtocolEncoding::Utf8, 64, 222, 222),
            "\x1b[M`ÿÿ".as_bytes()
        );
        assert_eq!(
            wheel_report(MouseProtocolEncoding::Utf8, 64, 2014, 2014),
            "\x1b[M`\u{7ff}\u{7ff}".as_bytes()
        );
        assert!(wheel_report(MouseProtocolEncoding::Utf8, 64, 0, 2015).is_empty());
        assert_eq!(
            wheel_report(MouseProtocolEncoding::Sgr, 64, 500, 1000),
            b"\x1b[<64;1001;501M"
        );
    }

    #[test]
    fn ordinary_history_shift_override_and_application_exit_restore_scrolling() {
        let mut parser = vt100::Parser::new(3, 20, 2000);
        for i in 0..20 {
            parser.process(format!("line-{i}\r\n").as_bytes());
        }
        let mut mouse = TerminalMouse::default();
        assert!(
            mouse
                .wheel(parser.screen_mut(), 1.0, KeyModifiers::empty())
                .is_empty()
        );
        assert_eq!(parser.screen().scrollback(), 3);
        parser.screen_mut().set_scrollback(0);
        parser.process(b"\x1b[?1003h\x1b[?1006h");
        assert!(
            mouse
                .wheel(parser.screen_mut(), 1.0, KeyModifiers::SHIFT)
                .is_empty()
        );
        assert_eq!(parser.screen().scrollback(), 3);
        parser.screen_mut().set_scrollback(0);
        parser.process(b"\x1b[?1049h");
        assert_eq!(
            mouse.wheel(parser.screen_mut(), 1.0, KeyModifiers::empty()),
            b"\x1b[<64;1;1M"
        );
        parser.process(b"\x1b[?1003l\x1b[?1006l\x1b[?1049l");
        assert!(
            mouse
                .wheel(parser.screen_mut(), 1.0, KeyModifiers::empty())
                .is_empty()
        );
        assert_eq!(parser.screen().scrollback(), 3);
        mouse.wheel(parser.screen_mut(), -1.0, KeyModifiers::empty());
        assert_eq!(parser.screen().scrollback(), 0);
        parser.process(b"\x1b[?9h");
        assert!(
            mouse
                .wheel(parser.screen_mut(), 1.0, KeyModifiers::empty())
                .is_empty()
        );
        assert_eq!(parser.screen().scrollback(), 3);
    }

    #[test]
    fn fractional_wheels_accumulate_and_reset_on_direction_or_mode_changes() {
        let mut parser = vt100::Parser::new(3, 20, 2000);
        parser.process(b"\x1b[?1003h\x1b[?1006h");
        let mut mouse = TerminalMouse::default();
        mouse.set_position(u16::MAX, u16::MAX);
        for _ in 0..3 {
            assert!(
                mouse
                    .wheel(parser.screen_mut(), 0.25, KeyModifiers::empty())
                    .is_empty()
            );
        }
        assert_eq!(
            mouse.wheel(parser.screen_mut(), 0.25, KeyModifiers::empty()),
            b"\x1b[<64;20;3M"
        );
        mouse.wheel(parser.screen_mut(), 0.75, KeyModifiers::empty());
        assert!(
            mouse
                .wheel(parser.screen_mut(), -0.25, KeyModifiers::empty())
                .is_empty()
        );
        assert_eq!(
            mouse.wheel(parser.screen_mut(), -0.75, KeyModifiers::empty()),
            b"\x1b[<65;20;3M"
        );
        mouse.wheel(parser.screen_mut(), 0.75, KeyModifiers::empty());
        parser.process(b"\x1b[?1003l");
        mouse.wheel(parser.screen_mut(), 0.25, KeyModifiers::empty());
        parser.process(b"\x1b[?1003h");
        assert!(
            mouse
                .wheel(parser.screen_mut(), 0.25, KeyModifiers::empty())
                .is_empty()
        );
        assert!(
            mouse
                .wheel(parser.screen_mut(), f32::NAN, KeyModifiers::empty())
                .is_empty()
        );
    }
}
