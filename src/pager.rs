mod terminal;

use std::io::{self, Write};

use console::{AnsiCodeIterator, Key, Term, measure_text_width};

const CONTROLS: &str = "Up/Down scroll | PgUp/PgDn page | Home/End | Tab/Shift+Tab sections | c finish | q/Esc/Ctrl+C cancel";

pub(crate) struct Document {
    pub(crate) text: String,
    pub(crate) sections: [usize; 2],
    pub(crate) root: String,
    pub(crate) status: &'static str,
    pub(crate) warning: bool,
}

struct Page {
    lines: Vec<String>,
    source_lines: Vec<usize>,
    sections: [usize; 2],
    header: Vec<String>,
    width: usize,
    height: usize,
}

impl Document {
    fn page(&self, size: (u16, u16)) -> io::Result<Page> {
        // Reserve the last column to avoid a terminal's implicit line wrap.
        let width = usize::from(size.1).saturating_sub(1);
        if width < 2 {
            return Err(io::Error::other("Terminal is too narrow for the viewer"));
        }
        let mut header = wrap_line(self.status, width);
        header.extend(wrap_line(&format!("Root: {}", self.root), width));
        if self.warning {
            header.extend(wrap_line(
                "WARNING: No keep matches. Everything under the root is a deletion target.",
                width,
            ));
        }
        let footer = wrap_line(CONTROLS, width);
        // One status line and a final spare row prevent scrolling the viewport.
        let reserved = header.len() + footer.len() + 2;
        let height = usize::from(size.0).saturating_sub(reserved);
        if height == 0 {
            return Err(io::Error::other("Terminal is too short for the viewer"));
        }
        let mut lines = Vec::new();
        let mut source_lines = Vec::new();
        let mut sections = [0; 2];
        for (source, line) in self.text.lines().enumerate() {
            for (index, start) in self.sections.iter().enumerate() {
                if source == *start {
                    sections[index] = lines.len();
                }
            }
            let wrapped = wrap_line(line, width);
            source_lines.extend(std::iter::repeat_n(source, wrapped.len()));
            lines.extend(wrapped);
        }
        Ok(Page {
            lines,
            source_lines,
            sections,
            header,
            width,
            height,
        })
    }
}

fn wrap_line(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut columns = 0;
    let mut style = String::new();
    for (part, ansi) in AnsiCodeIterator::new(text) {
        if ansi {
            line.push_str(part);
            if part == "\x1b[0m" || part == "\x1b[m" {
                style.clear();
            } else {
                style.push_str(part);
            }
            continue;
        }
        for character in part.chars() {
            let mut encoded = [0; 4];
            let cells = measure_text_width(character.encode_utf8(&mut encoded));
            if cells > 0 && columns + cells > width {
                if !style.is_empty() {
                    line.push_str("\x1b[0m");
                }
                lines.push(line);
                line = style.clone();
                columns = 0;
            }
            line.push(character);
            columns += cells;
        }
    }
    if !style.is_empty() {
        line.push_str("\x1b[0m");
    }
    lines.push(line);
    lines
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ViewerOutcome {
    Finished,
    Cancelled,
}

#[derive(Default)]
struct Position {
    top: usize,
}

impl Position {
    fn key(&mut self, key: Key, page: &Page) -> Option<ViewerOutcome> {
        let last = page.lines.len().saturating_sub(page.height);
        match key {
            Key::Char('c') => return Some(ViewerOutcome::Finished),
            Key::Char('q') | Key::Escape | Key::CtrlC => return Some(ViewerOutcome::Cancelled),
            Key::ArrowUp => self.top = self.top.saturating_sub(1),
            Key::ArrowDown => self.top = self.top.saturating_add(1).min(last),
            Key::PageUp => self.top = self.top.saturating_sub(page.height),
            Key::PageDown => self.top = self.top.saturating_add(page.height).min(last),
            Key::Home => self.top = 0,
            Key::End => self.top = last,
            Key::Tab => {
                self.top = page
                    .sections
                    .iter()
                    .copied()
                    .find(|start| *start > self.top)
                    .unwrap_or(page.sections[0])
                    .min(last);
            }
            Key::BackTab => {
                self.top = page
                    .sections
                    .iter()
                    .rev()
                    .copied()
                    .find(|start| *start < self.top)
                    .unwrap_or(page.sections[1])
                    .min(last);
            }
            _ => {}
        }
        None
    }
}

pub(crate) struct Viewer {
    term: Term,
    input: terminal::Terminal,
    page: Page,
    size: (u16, u16),
}

impl Viewer {
    pub(crate) fn prepare(document: &Document) -> io::Result<Self> {
        let term = Term::stdout();
        if !term.features().is_attended() || console::is_dumb() {
            return Err(io::Error::other(
                "Standard output is not a supported interactive terminal",
            ));
        }
        let size = term
            .size_checked()
            .ok_or_else(|| io::Error::other("Cannot determine terminal size"))?;
        let page = document.page(size)?;
        let input = terminal::Terminal::open()?;
        Ok(Self {
            term,
            input,
            page,
            size,
        })
    }

    pub(crate) fn needs_paging(&self) -> bool {
        self.page.lines.len() > self.page.height
    }

    pub(crate) fn show(mut self, document: &Document) -> io::Result<ViewerOutcome> {
        let result = self.show_inner(document);
        // Always attempt both restorations, even if the first operation fails.
        let input_result = self.input.finish();
        let cursor_result = self.term.show_cursor();
        match (result, input_result, cursor_result) {
            (result, Ok(()), Ok(())) => result,
            (result, input, cursor) => Err(io::Error::other(format!(
                "Viewer exit failed: operation={result:?}, input={input:?}, cursor={cursor:?}"
            ))),
        }
    }

    fn show_inner(&mut self, document: &Document) -> io::Result<ViewerOutcome> {
        self.input.start()?;
        self.term.hide_cursor()?;
        let mut position = Position::default();
        self.draw(&position)?;
        loop {
            let key = self.input.read_key()?;
            let size = self
                .term
                .size_checked()
                .ok_or_else(|| io::Error::other("Cannot determine terminal size"))?;
            let resized = size != self.size;
            if resized {
                let source = self
                    .page
                    .source_lines
                    .get(position.top)
                    .copied()
                    .unwrap_or(0);
                self.page = document.page(size)?;
                position.top = self
                    .page
                    .source_lines
                    .iter()
                    .position(|line| *line >= source)
                    .unwrap_or(0)
                    .min(self.page.lines.len().saturating_sub(self.page.height));
                self.size = size;
            }
            if let Some(key) = key {
                if let Some(outcome) = position.key(key, &self.page) {
                    self.term.clear_screen()?;
                    return Ok(outcome);
                }
                self.draw(&position)?;
            } else if resized {
                self.draw(&position)?;
            }
        }
    }

    fn draw(&self, position: &Position) -> io::Result<()> {
        self.term.clear_screen()?;
        let mut screen = String::new();
        for line in &self.page.header {
            screen.push_str(line);
            screen.push('\n');
        }
        for row in 0..self.page.height {
            if let Some(line) = self.page.lines.get(position.top + row) {
                screen.push_str(line);
            }
            screen.push('\n');
        }
        let section = if position.top >= self.page.sections[1] {
            "DELETE"
        } else {
            "KEEP"
        };
        let status = format!(
            "{section} | lines {}-{} / {}",
            position.top + 1,
            (position.top + self.page.height).min(self.page.lines.len()),
            self.page.lines.len()
        );
        screen.push_str(&console::truncate_str(&status, self.page.width, ""));
        screen.push('\n');
        for line in wrap_line(CONTROLS, self.page.width) {
            screen.push_str(&line);
            screen.push('\n');
        }
        let mut output = io::stdout().lock();
        output.write_all(screen.as_bytes())?;
        output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(text: &str) -> Document {
        Document {
            text: text.to_owned(),
            sections: [0, 2],
            root: "test".to_owned(),
            status: "PREVIEW ONLY",
            warning: false,
        }
    }

    #[test]
    fn wraps_without_losing_wide_or_combining_characters() {
        for text in ["abcdefghi", "日本語の長い名前", "e\u{301}e\u{301}e\u{301}"] {
            let lines = wrap_line(text, 4);
            assert_eq!(lines.concat(), text);
            assert!(lines.iter().all(|line| measure_text_width(line) <= 4));
        }
        assert_eq!(wrap_line("abcd", 4), ["abcd"]);
        assert_eq!(wrap_line("abcde", 4), ["abcd", "e"]);
    }

    #[test]
    fn wrapped_colors_are_independently_reset() {
        let lines = wrap_line("\x1b[34mabcdef\x1b[0m", 3);
        assert_eq!(lines, ["\x1b[34mabc\x1b[0m", "\x1b[34mdef\x1b[0m"]);
    }

    #[test]
    fn fit_uses_wrapped_rows_and_reserved_space() {
        let short = document("a\nb\nc\nd\n");
        let page = short.page((10, 100)).unwrap();
        assert_eq!(page.height, 4);
        assert_eq!(short.page((11, 100)).unwrap().height, 5);
        assert_eq!(short.page((9, 100)).unwrap().height, 3);
        let long = document(&"x".repeat(100));
        assert_eq!(long.page((10, 100)).unwrap().lines.len(), 2);
        assert!(long.page((2, 100)).is_err());
        assert!(long.page((20, 2)).is_err());
    }

    #[test]
    fn navigation_and_completion_do_not_confirm_deletion() {
        let doc = document("a\nb\nc\nd\ne\nf\ng\nh\n");
        let page = doc.page((10, 100)).unwrap();
        let mut position = Position::default();
        for key in [Key::Enter, Key::Char('y'), Key::Unknown] {
            assert_eq!(position.key(key, &page), None);
        }
        position.key(Key::PageDown, &page);
        assert_eq!(position.top, 4);
        position.key(Key::End, &page);
        assert_eq!(position.top, 4);
        position.key(Key::Home, &page);
        position.key(Key::Tab, &page);
        assert_eq!(position.top, 2);
        position.key(Key::BackTab, &page);
        assert_eq!(position.top, 0);
        assert_eq!(
            position.key(Key::Char('c'), &page),
            Some(ViewerOutcome::Finished)
        );
        for key in [Key::Char('q'), Key::Escape, Key::CtrlC] {
            assert_eq!(position.key(key, &page), Some(ViewerOutcome::Cancelled));
        }
    }
}
