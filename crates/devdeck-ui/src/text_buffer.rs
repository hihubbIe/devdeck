//! Editing logic of the single-line text input, with no UI so it can be unit tested.
//!
//! Offsets are byte offsets into the UTF-8 text and always sit on grapheme boundaries
//! for caret moves (an emoji or an accented letter is one step). GPUI's input handler
//! speaks UTF-16 offsets; [`TextBuffer::utf16_to_byte`] and
//! [`TextBuffer::byte_to_utf16`] convert. The IME composition is the `marked` range.

use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextBuffer {
    text: String,
    /// Selection as an anchor and a head; the caret is the head.
    anchor: usize,
    head: usize,
    /// Text being composed by an input method, underlined while it lasts.
    marked: Option<Range<usize>>,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    /// End of the last single-character insert, to group typing into one undo step.
    typing_at: Option<usize>,
}

/// Single-line input: line breaks and tabs become spaces.
fn normalize(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\n', '\r', '\t'], " ")
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self { text, anchor: end, head: end, ..Default::default() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.head
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    fn clamp(&self, i: usize) -> usize {
        let mut i = i.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    fn prev_char(&self, i: usize) -> usize {
        self.text[..i].grapheme_indices(true).next_back().map_or(0, |(p, _)| p)
    }

    fn next_char(&self, i: usize) -> usize {
        self.text[i..].graphemes(true).next().map_or(i, |g| i + g.len())
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// The word around `i` (for double-click), or an empty range on whitespace.
    pub fn word_at(&self, i: usize) -> Range<usize> {
        let i = self.clamp(i);
        for (start, w) in self.text.split_word_bound_indices() {
            if (start..start + w.len()).contains(&i) || (i == self.text.len() && start + w.len() == i) {
                return if w.chars().any(is_word) { start..start + w.len() } else { i..i };
            }
        }
        i..i
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.anchor = self.clamp(range.start);
        self.head = self.clamp(range.end);
    }

    fn prev_word(&self, i: usize) -> usize {
        let mut it = self.text[..i].char_indices().rev().peekable();
        while it.peek().is_some_and(|(_, c)| !is_word(*c)) {
            it.next();
        }
        let mut pos = it.peek().map_or(0, |(p, _)| *p);
        for (p, c) in it {
            if !is_word(c) {
                break;
            }
            pos = p;
        }
        if self.text[..i].chars().all(|c| !is_word(c)) { 0 } else { pos }
    }

    fn next_word(&self, i: usize) -> usize {
        let rest = &self.text[i..];
        let mut it = rest.char_indices().peekable();
        while it.peek().is_some_and(|(_, c)| !is_word(*c)) {
            it.next();
        }
        while it.peek().is_some_and(|(_, c)| is_word(*c)) {
            it.next();
        }
        i + it.peek().map_or(rest.len(), |(p, _)| *p)
    }

    /// Moves the caret; with `select` the anchor stays, extending the selection.
    pub fn move_to(&mut self, i: usize, select: bool) {
        self.head = self.clamp(i);
        if !select {
            self.anchor = self.head;
        }
    }

    pub fn left(&mut self, select: bool) {
        let to = if self.has_selection() && !select { self.selection().start } else { self.prev_char(self.head) };
        self.move_to(to, select);
    }

    pub fn right(&mut self, select: bool) {
        let to = if self.has_selection() && !select { self.selection().end } else { self.next_char(self.head) };
        self.move_to(to, select);
    }

    pub fn word_left(&mut self, select: bool) {
        self.move_to(self.prev_word(self.head), select);
    }

    pub fn word_right(&mut self, select: bool) {
        self.move_to(self.next_word(self.head), select);
    }

    pub fn home(&mut self, select: bool) {
        self.move_to(0, select);
    }

    pub fn end(&mut self, select: bool) {
        self.move_to(self.text.len(), select);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.text.len();
    }

    fn snapshot(&mut self) {
        if self.undo.last().is_none_or(|(t, _)| *t != self.text) {
            self.undo.push((self.text.clone(), self.head));
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }

    /// Replaces `range` (byte offsets, clamped) with `s` and puts the caret after it.
    /// Consecutive single-character typing is one undo step.
    pub fn replace(&mut self, range: Range<usize>, s: &str) {
        let (a, b) = (self.clamp(range.start), self.clamp(range.end));
        let (a, b) = (a.min(b), a.max(b));
        let s = normalize(s);
        let typing = a == b && s.graphemes(true).count() == 1 && self.typing_at == Some(a);
        // Committing a composition: its start was already recorded.
        let committing = self.marked.is_some();
        if !typing && !committing {
            self.snapshot();
        }
        self.text.replace_range(a..b, &s);
        self.marked = None;
        self.move_to(a + s.len(), false);
        self.typing_at = (s.graphemes(true).count() == 1 && a == b).then_some(a + s.len());
    }

    /// IME composition: replaces `range` with `s`, marks it, and selects
    /// `selected` (relative to `s`, or the end of `s`). Only the start of a composition
    /// is an undo step.
    pub fn replace_and_mark(&mut self, range: Range<usize>, s: &str, selected: Option<Range<usize>>) {
        let (a, b) = (self.clamp(range.start), self.clamp(range.end));
        let (a, b) = (a.min(b), a.max(b));
        if self.marked.is_none() {
            self.snapshot();
        }
        let s = normalize(s);
        self.text.replace_range(a..b, &s);
        self.marked = (!s.is_empty()).then_some(a..a + s.len());
        let sel = selected.map_or(s.len()..s.len(), |r| r.start.min(s.len())..r.end.min(s.len()));
        self.anchor = self.clamp(a + sel.start);
        self.head = self.clamp(a + sel.end);
        self.typing_at = None;
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Types `s` over the selection.
    pub fn insert(&mut self, s: &str) {
        self.replace(self.selection(), s);
    }

    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.insert("");
        } else if self.head > 0 {
            let p = self.prev_char(self.head);
            self.replace(p..self.head, "");
        }
    }

    pub fn delete(&mut self) {
        if self.has_selection() {
            self.insert("");
        } else if self.head < self.text.len() {
            let n = self.next_char(self.head);
            self.replace(self.head..n, "");
        }
    }

    pub fn delete_word_left(&mut self) {
        if self.has_selection() {
            self.insert("");
        } else {
            let p = self.prev_word(self.head);
            self.replace(p..self.head, "");
        }
    }

    pub fn undo(&mut self) {
        if let Some((text, caret)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.text, text), self.head));
            self.marked = None;
            self.typing_at = None;
            self.move_to(caret, false);
        }
    }

    pub fn redo(&mut self) {
        if let Some((text, caret)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.text, text), self.head));
            self.marked = None;
            self.typing_at = None;
            self.move_to(caret, false);
        }
    }

    /// Replaces the whole text (programmatic set), keeping undo history.
    pub fn set_text(&mut self, s: &str) {
        self.replace(0..self.text.len(), s);
    }

    pub fn utf16_to_byte(&self, utf16: usize) -> usize {
        let mut count = 0;
        for (i, c) in self.text.char_indices() {
            if count >= utf16 {
                return i;
            }
            count += c.len_utf16();
        }
        self.text.len()
    }

    pub fn byte_to_utf16(&self, byte: usize) -> usize {
        self.text[..self.clamp(byte)].chars().map(char::len_utf16).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_caret() {
        let mut b = TextBuffer::new("");
        b.insert("hello");
        assert_eq!((b.text(), b.caret()), ("hello", 5));
        b.left(false);
        b.left(false);
        b.insert("X");
        assert_eq!(b.text(), "helXlo");
        b.home(false);
        b.insert(">");
        b.end(false);
        b.insert("<");
        assert_eq!(b.text(), ">helXlo<");
    }

    #[test]
    fn selection_replace_and_collapse() {
        let mut b = TextBuffer::new("abcdef");
        b.home(false);
        b.right(true);
        b.right(true);
        assert_eq!(b.selected_text(), "ab");
        b.right(false);
        assert_eq!((b.caret(), b.has_selection()), (2, false), "right collapses to the selection end");
        b.select_all();
        b.insert("z");
        assert_eq!(b.text(), "z");
    }

    #[test]
    fn backspace_delete_and_words() {
        let mut b = TextBuffer::new("kube-system dev35");
        b.delete_word_left();
        assert_eq!(b.text(), "kube-system ");
        b.word_left(false);
        assert_eq!(b.caret(), 5, "word left stops at 'system'");
        b.word_left(false);
        assert_eq!(b.caret(), 0);
        b.word_right(false);
        assert_eq!(b.caret(), 4);
        b.delete();
        assert_eq!(b.text(), "kubesystem ");
        b.end(false);
        b.backspace();
        assert_eq!(b.text(), "kubesystem");
    }

    #[test]
    fn utf8_safe_and_utf16_conversion() {
        let mut b = TextBuffer::new("é›🚀a");
        b.left(false);
        b.left(false);
        assert_eq!(&b.text()[b.caret()..], "🚀a");
        b.backspace();
        assert_eq!(b.text(), "é🚀a");
        assert_eq!(b.byte_to_utf16(b.text().len()), 1 + 2 + 1);
        assert_eq!(b.utf16_to_byte(1), 2);
        assert_eq!(b.utf16_to_byte(3), 6);
        b.move_to(3, false);
        assert_eq!(b.caret(), 2, "offsets snap to char boundaries");
    }

    #[test]
    fn undo_groups_typing_and_redo_works() {
        let mut b = TextBuffer::new("ab");
        for c in ["c", "d", "e"] {
            b.insert(c);
        }
        b.insert("\tx\r\ny");
        assert_eq!(b.text(), "abcde x y", "tabs and line breaks become spaces");
        b.undo();
        assert_eq!(b.text(), "abcde");
        b.undo();
        assert_eq!(b.text(), "ab", "typing c, d, e was one step");
        b.redo();
        assert_eq!(b.text(), "abcde");
        b.redo();
        assert_eq!(b.text(), "abcde x y");
        b.undo();
        b.insert("Z");
        b.redo();
        assert_eq!(b.text(), "abcdeZ", "a new edit clears redo");
    }

    #[test]
    fn graphemes_and_words() {
        let mut b = TextBuffer::new("e\u{301}x");
        b.home(false);
        b.right(false);
        assert_eq!(b.caret(), 3, "e + combining accent is one step");
        let b = TextBuffer::new("kube-system dev35");
        assert_eq!(b.word_at(6), 5..11);
        assert_eq!(b.word_at(11), 11..11, "space is not a word");
    }

    #[test]
    fn ime_composition() {
        let mut b = TextBuffer::new("ab");
        b.replace_and_mark(2..2, "ka", None);
        assert_eq!((b.text(), b.marked()), ("abka", Some(2..4)));
        b.replace_and_mark(2..4, "か", Some(0..0));
        assert_eq!((b.text(), b.marked(), b.caret()), ("abか", Some(2..5), 2));
        b.replace(2..5, "家");
        assert_eq!((b.text(), b.marked()), ("ab家", None));
        b.undo();
        assert_eq!(b.text(), "ab", "the whole composition undoes at once");
    }
}
