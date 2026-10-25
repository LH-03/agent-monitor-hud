//! Text box editing model: caret, selection, clipboard. Characters arrive as
//! WM_CHAR, so IME composition results work without special handling.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

const CF_UNICODETEXT: u32 = 13;

#[derive(Clone, Debug, Default)]
pub struct TextEdit {
    pub chars: Vec<char>,
    pub caret: usize,
    pub anchor: usize,
    pub multiline: bool,
    pub max_len: usize,
    /// Scroll offset in DIPs (horizontal for single line, vertical for multi-line).
    pub scroll: f32,
    pub dirty: bool,
}

impl TextEdit {
    pub fn new(text: &str, multiline: bool, max_len: usize) -> TextEdit {
        let chars: Vec<char> = text.chars().collect();
        TextEdit { caret: chars.len(), anchor: chars.len(), chars, multiline, max_len, scroll: 0.0, dirty: false }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.caret = self.caret.min(self.chars.len());
        self.anchor = self.caret;
        self.dirty = false;
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        if a == b {
            return false;
        }
        self.chars.drain(a..b);
        self.caret = a;
        self.anchor = a;
        self.dirty = true;
        true
    }

    pub fn insert(&mut self, text: &str) {
        self.delete_selection();
        let mut incoming: Vec<char> = text
            .chars()
            .filter(|c| if self.multiline { *c == '\n' || !c.is_control() } else { !c.is_control() })
            .collect();
        let room = self.max_len.saturating_sub(self.chars.len());
        incoming.truncate(room);
        let n = incoming.len();
        self.chars.splice(self.caret..self.caret, incoming);
        self.caret += n;
        self.anchor = self.caret;
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if !self.delete_selection() && self.caret > 0 {
            self.caret -= 1;
            self.chars.remove(self.caret);
            self.anchor = self.caret;
            self.dirty = true;
        }
    }

    pub fn delete(&mut self) {
        if !self.delete_selection() && self.caret < self.chars.len() {
            self.chars.remove(self.caret);
            self.dirty = true;
        }
    }

    pub fn move_to(&mut self, position: usize, extend: bool) {
        self.caret = position.min(self.chars.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn left(&mut self, extend: bool, word: bool) {
        if self.has_selection() && !extend {
            let (a, _) = self.selection();
            self.move_to(a, false);
            return;
        }
        let mut p = self.caret;
        if word {
            while p > 0 && self.chars[p - 1].is_whitespace() {
                p -= 1;
            }
            while p > 0 && !self.chars[p - 1].is_whitespace() {
                p -= 1;
            }
        } else {
            p = p.saturating_sub(1);
        }
        self.move_to(p, extend);
    }

    pub fn right(&mut self, extend: bool, word: bool) {
        if self.has_selection() && !extend {
            let (_, b) = self.selection();
            self.move_to(b, false);
            return;
        }
        let mut p = self.caret;
        let n = self.chars.len();
        if word {
            while p < n && !self.chars[p].is_whitespace() {
                p += 1;
            }
            while p < n && self.chars[p].is_whitespace() {
                p += 1;
            }
        } else {
            p = (p + 1).min(n);
        }
        self.move_to(p, extend);
    }

    pub fn home(&mut self, extend: bool) {
        let p = if self.multiline { self.chars[..self.caret].iter().rposition(|c| *c == '\n').map(|i| i + 1).unwrap_or(0) } else { 0 };
        self.move_to(p, extend);
    }

    pub fn end(&mut self, extend: bool) {
        let p = if self.multiline {
            self.chars[self.caret..].iter().position(|c| *c == '\n').map(|i| self.caret + i).unwrap_or(self.chars.len())
        } else {
            self.chars.len()
        };
        self.move_to(p, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.chars.len();
    }

    pub fn copy(&self) {
        let (a, b) = self.selection();
        if a != b {
            set_clipboard(&self.chars[a..b].iter().collect::<String>());
        }
    }

    pub fn cut(&mut self) {
        self.copy();
        self.delete_selection();
    }

    pub fn paste(&mut self) {
        if let Some(text) = get_clipboard() {
            let text = if self.multiline { text.replace("\r\n", "\n") } else { text.replace(['\r', '\n'], " ") };
            self.insert(&text);
        }
    }

    /// UTF-16 offset of a char index (DirectWrite positions are UTF-16).
    pub fn utf16_index(&self, index: usize) -> u32 {
        self.chars[..index.min(self.chars.len())].iter().map(|c| c.len_utf16() as u32).sum()
    }

    pub fn char_index(&self, utf16: u32) -> usize {
        let mut total = 0u32;
        for (i, c) in self.chars.iter().enumerate() {
            if total >= utf16 {
                return i;
            }
            total += c.len_utf16() as u32;
        }
        self.chars.len()
    }
}

fn set_clipboard(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            let pointer = GlobalLock(memory) as *mut u16;
            if !pointer.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
                let _ = GlobalUnlock(memory);
                if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).is_err() {
                    let _ = GlobalFree(Some(memory));
                }
            }
        }
        let _ = CloseClipboard();
    }
}

fn get_clipboard() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let result = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
            let memory = HGLOBAL(handle.0);
            let pointer = GlobalLock(memory) as *const u16;
            if pointer.is_null() {
                return None;
            }
            let mut len = 0;
            while *pointer.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, len));
            let _ = GlobalUnlock(memory);
            Some(text)
        })();
        let _ = CloseClipboard();
        result
    }
}
