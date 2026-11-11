//! Champ de saisie texte minimal (une ou plusieurs lignes), indépendant du rendu.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    /// Position du curseur en caractères (pas en octets).
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map_or(self.value.len(), |(b, _)| b)
    }

    pub fn insert(&mut self, c: char) {
        let b = self.byte_index(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
    }

    /// Insère un texte collé. Une ligne : sauts de ligne (et tabulations) → espace.
    /// Multiligne : fins de ligne normalisées en `\n`. Autres caractères de contrôle ignorés.
    pub fn insert_str(&mut self, text: &str, multiline: bool) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        for c in normalized.chars() {
            match c {
                '\n' if multiline => self.insert('\n'),
                '\n' | '\t' => self.insert(' '),
                c if c.is_control() => {}
                c => self.insert(c),
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let b = self.byte_index(self.cursor);
            self.value.remove(b);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let b = self.byte_index(self.cursor);
            self.value.remove(b);
        }
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Traite une touche d'édition. Renvoie `true` si la touche a été consommée.
    /// En mode multiligne, Entrée insère un saut de ligne.
    pub fn handle_key(&mut self, key: KeyEvent, multiline: bool) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u') if ctrl => self.clear(),
            KeyCode::Char(_) if ctrl => return false,
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Enter if multiline => self.insert('\n'),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => return false,
        }
        true
    }

    /// (ligne, colonne) du curseur, en caractères.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before: String = self.value.chars().take(self.cursor).collect();
        let line = before.matches('\n').count();
        let col = before.rsplit('\n').next().map_or(0, |s| s.chars().count());
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_with_multibyte_chars() {
        let mut t = TextInput::new("été");
        t.handle_key(key(KeyCode::Left), false);
        t.handle_key(key(KeyCode::Char('x')), false);
        assert_eq!(t.value(), "étxé");
        t.handle_key(key(KeyCode::Home), false);
        t.handle_key(key(KeyCode::Delete), false);
        assert_eq!(t.value(), "txé");
        t.handle_key(key(KeyCode::End), false);
        t.handle_key(key(KeyCode::Backspace), false);
        assert_eq!(t.value(), "tx");
        assert!(!t.handle_key(key(KeyCode::Enter), false));
        assert!(t.handle_key(key(KeyCode::Enter), true));
        t.insert('y');
        assert_eq!(t.cursor_line_col(), (1, 1));
        t.handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            false,
        );
        assert_eq!(t.value(), "");
    }

    #[test]
    fn paste_single_and_multi_line() {
        let mut t = TextInput::new("a");
        t.insert_str("Corriger le bug\r\nvoir log\n", false);
        assert_eq!(t.value(), "aCorriger le bug voir log ");
        let mut m = TextInput::new("");
        m.insert_str("l1\r\nl2\rl3\x1b", true);
        assert_eq!(m.value(), "l1\nl2\nl3");
    }
}
