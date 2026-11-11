//! Shared measured geometry for proportional and fixed-width editor fonts.
use super::Metrics;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

impl Metrics {
    /// Pixel positions at character boundaries. Graphemes share one shaped advance.
    pub(super) fn positions(&self, text: &str, tab: usize) -> Arc<Vec<f32>> {
        let key = (text.to_owned(), tab);
        if let Some(value) = self.geometry.borrow().get(&key) {
            return value.clone();
        }
        let tab_width = (self.char_w * tab.max(1) as f32).max(1.0);
        let mut positions = vec![0.0];
        let mut x = 0.0;
        for grapheme in text.graphemes(true) {
            let count = grapheme.chars().count();
            let width = if grapheme == "\t" {
                ((x / tab_width).floor() + 1.0) * tab_width - x
            } else {
                self.ctx.fonts_mut(|f| {
                    f.layout_no_wrap(grapheme.to_owned(), self.font.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
            };
            for _ in 1..count {
                positions.push(x);
            }
            x += width;
            positions.push(x);
        }
        let positions = Arc::new(positions);
        // Bound the cache on a frame containing very many wrapped lines.
        if self.geometry.borrow().len() < 4096 {
            self.geometry.borrow_mut().insert(key, positions.clone());
        }
        positions
    }

    pub(super) fn x_of(&self, text: &str, chars: usize, tab: usize) -> f32 {
        let positions = self.positions(text, tab);
        positions[chars.min(positions.len() - 1)]
    }

    pub(super) fn char_at_x(&self, text: &str, x: f32, tab: usize) -> usize {
        let positions = self.positions(text, tab);
        let mut chars = 0;
        for g in text.graphemes(true) {
            let end = chars + g.chars().count();
            if x < (positions[chars] + positions[end]) * 0.5 {
                return chars;
            }
            if x < positions[end] {
                return end;
            }
            chars = end;
        }
        chars
    }

    pub(super) fn x_at_col(&self, text: &str, col: usize, tab: usize) -> f32 {
        use notoir_editor::wrap::{char_at_col, col_of};
        let at = char_at_col(text, col, tab);
        let total = col_of(text, text.chars().count(), tab);
        self.x_of(text, at, tab) + col.saturating_sub(total) as f32 * self.char_w
    }

    pub(super) fn wrap_starts(&self, text: &str, width: f32, tab: usize) -> Vec<usize> {
        let mut starts = vec![0];
        let mut row_x: f32 = 0.0;
        let mut count = 0;
        let mut last_space = None;
        let graphemes: Vec<_> = text.graphemes(true).collect();
        let mut i = 0;
        while i < graphemes.len() {
            let g = graphemes[i];
            let tab_width = (self.char_w * tab.max(1) as f32).max(1.0);
            let advance = if g == "\t" {
                ((row_x / tab_width).floor() + 1.0) * tab_width - row_x
            } else {
                self.x_of(g, g.chars().count(), tab)
            };
            if row_x > 0.0 && !g.chars().all(char::is_whitespace) && row_x + advance > width {
                let (break_i, break_chars) = last_space
                    .filter(|(_, c)| *c > *starts.last().unwrap_or(&0))
                    .unwrap_or((i, count));
                starts.push(break_chars);
                i = break_i;
                count = break_chars;
                last_space = None;
                row_x = 0.0;
                continue;
            }
            row_x += advance;
            count += g.chars().count();
            i += 1;
            if g.chars().all(char::is_whitespace) {
                last_space = Some((i, count));
            }
        }
        starts
    }
}
