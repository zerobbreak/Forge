//! A minimal multi-line text buffer with a cursor, for TUI input fields.
//! Columns count characters, not bytes.

use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextArea {
    lines: Vec<String>,
    row: usize,
    column: usize,
    /// Single-line fields ignore `Enter` and keep everything on one row.
    single_line: bool,
}

impl TextArea {
    pub fn single_line(text: &str) -> Self {
        let mut area = Self::multi_line("");
        area.single_line = true;
        area.set_text(text);
        area
    }

    pub fn multi_line(text: &str) -> Self {
        let mut area = Self {
            lines: vec![String::new()],
            row: 0,
            column: 0,
            single_line: false,
        };
        area.set_text(text);
        area
    }

    /// Replaces the content and moves the cursor to its end.
    pub fn set_text(&mut self, text: &str) {
        self.lines = if self.single_line {
            vec![text.replace(['\r', '\n'], " ")]
        } else {
            text.replace("\r\n", "\n")
                .split('\n')
                .map(str::to_owned)
                .collect()
        };
        self.row = self.lines.len() - 1;
        self.column = self.lines[self.row].chars().count();
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// `(row, column)` of the cursor.
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.column)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
    }

    /// Applies an editing key. Returns `false` when the key is not an editing key
    /// (or would move past the first/last row), so the caller can handle it.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        // Windows reports AltGr characters (`@`, `{` on many layouts) as Ctrl+Alt.
        let alt_gr = modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if !alt_gr && modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return false;
        }
        match code {
            KeyCode::Char(character) => self.insert(character),
            KeyCode::Enter if !self.single_line => self.newline(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.column = 0,
            KeyCode::End => self.column = self.line_length(),
            KeyCode::Up if self.row > 0 => {
                self.row -= 1;
                self.column = self.column.min(self.line_length());
            }
            KeyCode::Down if self.row + 1 < self.lines.len() => {
                self.row += 1;
                self.column = self.column.min(self.line_length());
            }
            _ => return false,
        }
        true
    }

    fn line_length(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    fn byte_index(&self, column: usize) -> usize {
        let line = &self.lines[self.row];
        line.char_indices()
            .nth(column)
            .map_or(line.len(), |(index, _)| index)
    }

    fn insert(&mut self, character: char) {
        let index = self.byte_index(self.column);
        self.lines[self.row].insert(index, character);
        self.column += 1;
    }

    fn newline(&mut self) {
        let index = self.byte_index(self.column);
        let rest = self.lines[self.row].split_off(index);
        self.row += 1;
        self.lines.insert(self.row, rest);
        self.column = 0;
    }

    fn backspace(&mut self) {
        if self.column > 0 {
            self.column -= 1;
            let index = self.byte_index(self.column);
            self.lines[self.row].remove(index);
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.line_length();
            self.lines[self.row].push_str(&line);
        }
    }

    fn delete(&mut self) {
        if self.column < self.line_length() {
            let index = self.byte_index(self.column);
            self.lines[self.row].remove(index);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.line_length();
        }
    }

    fn right(&mut self) {
        if self.column < self.line_length() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextArea;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn keys(area: &mut TextArea, codes: &[KeyCode]) {
        for code in codes {
            area.handle_key(*code, KeyModifiers::NONE);
        }
    }

    #[test]
    fn edits_across_lines_with_multibyte_text() {
        let mut area = TextArea::multi_line("");
        for character in "héllo".chars() {
            area.handle_key(KeyCode::Char(character), KeyModifiers::NONE);
        }
        keys(&mut area, &[KeyCode::Left, KeyCode::Left, KeyCode::Enter]);
        assert_eq!(area.text(), "hél\nlo");
        assert_eq!(area.cursor(), (1, 0));
        keys(&mut area, &[KeyCode::Backspace]);
        assert_eq!(area.text(), "hello".replace('e', "é"));
        keys(&mut area, &[KeyCode::Home, KeyCode::Delete]);
        assert_eq!(area.text(), "éllo");
    }

    #[test]
    fn single_line_rejects_enter_and_vertical_moves() {
        let mut area = TextArea::single_line("a\nb");
        assert_eq!(area.text(), "a b");
        assert!(!area.handle_key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!area.handle_key(KeyCode::Down, KeyModifiers::NONE));
        assert!(!area.handle_key(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(area.handle_key(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(area.text(), "a bc");
    }
}
