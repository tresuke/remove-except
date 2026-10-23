mod terminal;

use std::io::{self, Write};
use std::ops::Range;

use console::{AnsiCodeIterator, Key, Term, measure_text_width};
use unicode_segmentation::UnicodeSegmentation;

const CONTROLS: &str = "Up/Down scroll | PgUp/PgDn page | Home/End | Tab/Shift+Tab sections | c finish | q/Esc/Ctrl+C cancel";

pub(crate) struct Document {
    pub(crate) text: String,
    pub(crate) sections: [Range<usize>; 2],
    pub(crate) root: String,
    pub(crate) status: &'static str,
    pub(crate) warning: bool,
}

struct Page {
    lines: Vec<String>,
    source_lines: Vec<usize>,
    sections: [Range<usize>; 2],
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
        let mut header = wrap_line(self.status, width)?;
        header.extend(wrap_line(&format!("Root: {}", self.root), width)?);
        if self.warning {
            header.extend(wrap_line(
                "WARNING: No keep matches. Everything under the root is a deletion target.",
                width,
            )?);
        }
        let footer = wrap_line(CONTROLS, width)?;
        // One status line and a final spare row prevent scrolling the viewport.
        let reserved = header.len() + footer.len() + 2;
        let height = usize::from(size.0).saturating_sub(reserved);
        if height == 0 {
            return Err(io::Error::other("Terminal is too short for the viewer"));
        }
        let mut lines = Vec::new();
        let mut source_lines = Vec::new();
        let mut line_starts = Vec::new();
        for (source, line) in self.text.lines().enumerate() {
            line_starts.push(lines.len());
            let wrapped = wrap_line(line, width)?;
            source_lines.extend(std::iter::repeat_n(source, wrapped.len()));
            lines.extend(wrapped);
        }
        line_starts.push(lines.len());
        let mut sections = [0..0, 0..0];
        for (index, range) in self.sections.iter().enumerate() {
            let start = line_starts.get(range.start);
            let end = line_starts.get(range.end);
            match (start, end) {
                (Some(&start), Some(&end)) if start <= end => sections[index] = start..end,
                _ => return Err(io::Error::other("Invalid viewer section range")),
            }
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

fn wrap_line(text: &str, width: usize) -> io::Result<Vec<String>> {
    let mut plain = String::new();
    let mut offsets = Vec::new();
    let mut original_offset = 0;
    for (part, ansi) in AnsiCodeIterator::new(text) {
        if !ansi {
            for (index, character) in part.char_indices() {
                offsets.push((
                    plain.len() + index + character.len_utf8(),
                    original_offset + index + character.len_utf8(),
                ));
            }
            plain.push_str(part);
        }
        original_offset += part.len();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut visible = String::new();
    let mut style = String::new();
    let mut original_start = 0;
    let mut ends = offsets.into_iter().peekable();
    for (start, cluster) in plain.grapheme_indices(true) {
        if measure_text_width(cluster) > width {
            return Err(io::Error::other(
                "A grapheme cluster is wider than the available viewer row",
            ));
        }
        let previous_length = visible.len();
        visible.push_str(cluster);
        if measure_text_width(&visible) > width {
            if !style.is_empty() {
                line.push_str("\x1b[0m");
            }
            lines.push(line);
            line = style.clone();
            visible.drain(..previous_length);
        }
        let end = start + cluster.len();
        while ends.peek().is_some_and(|(plain_end, _)| *plain_end < end) {
            ends.next();
        }
        let (_, original_end) = ends
            .next()
            .ok_or_else(|| io::Error::other("Invalid viewer text offset"))?;
        append_styled(&mut line, &mut style, &text[original_start..original_end]);
        original_start = original_end;
    }
    append_styled(&mut line, &mut style, &text[original_start..]);
    if !style.is_empty() {
        line.push_str("\x1b[0m");
    }
    lines.push(line);
    Ok(lines)
}

fn append_styled(line: &mut String, style: &mut String, text: &str) {
    for (part, ansi) in AnsiCodeIterator::new(text) {
        line.push_str(part);
        if ansi {
            if part == "\x1b[0m" || part == "\x1b[m" {
                style.clear();
            } else {
                style.push_str(part);
            }
        }
    }
}

impl Page {
    fn status_line(&self, top: usize, colors_enabled: bool) -> String {
        let style = console::Style::new().bold().force_styling(colors_enabled);
        let section = match self.section_label(top) {
            "KEEP" => style.green().apply_to("KEEP").to_string(),
            "DELETE" => style.red().apply_to("DELETE").to_string(),
            "KEEP + DELETE" => format!(
                "{} + {}",
                style.clone().green().apply_to("KEEP"),
                style.red().apply_to("DELETE")
            ),
            _ => style.cyan().apply_to("PLAN").to_string(),
        };
        let status = format!(
            "{section} | lines {}-{} / {}",
            top + 1,
            (top + self.height).min(self.lines.len()),
            self.lines.len()
        );
        console::truncate_str(&status, self.width, "").into_owned()
    }

    fn section_label(&self, top: usize) -> &'static str {
        let end = top.saturating_add(self.height).min(self.lines.len());
        let visible = self
            .sections
            .each_ref()
            .map(|range| range.start < range.end && top < range.end && range.start < end);
        match visible {
            [true, true] => "KEEP + DELETE",
            [true, false] => "KEEP",
            [false, true] => "DELETE",
            [false, false] => "PLAN",
        }
    }
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
                let targets = page.sections.each_ref().map(|range| range.start.min(last));
                self.top = targets
                    .iter()
                    .copied()
                    .find(|start| *start > self.top)
                    .unwrap_or(targets[0]);
            }
            Key::BackTab => {
                let targets = page.sections.each_ref().map(|range| range.start.min(last));
                self.top = targets
                    .iter()
                    .rev()
                    .copied()
                    .find(|start| *start < self.top)
                    .unwrap_or(targets[1]);
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
        screen.push_str(
            &self
                .page
                .status_line(position.top, console::colors_enabled()),
        );
        screen.push('\n');
        for line in wrap_line(CONTROLS, self.page.width)? {
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
        let count = text.lines().count();
        let split = 2.min(count);
        Document {
            text: text.to_owned(),
            sections: [0..split, split..count],
            root: "test".to_owned(),
            status: "PREVIEW ONLY",
            warning: false,
        }
    }

    #[test]
    fn wraps_without_losing_wide_or_combining_characters() {
        for text in ["abcdefghi", "日本語の長い名前", "e\u{301}e\u{301}e\u{301}"] {
            let lines = wrap_line(text, 4).unwrap();
            assert_eq!(lines.concat(), text);
            assert!(lines.iter().all(|line| measure_text_width(line) <= 4));
        }
        assert_eq!(wrap_line("abcd", 4).unwrap(), ["abcd"]);
        assert_eq!(wrap_line("abcde", 4).unwrap(), ["abcd", "e"]);
    }

    #[test]
    fn wrapped_colors_are_independently_reset() {
        let lines = wrap_line("\x1b[34mabcdef\x1b[0m", 3).unwrap();
        assert_eq!(lines, ["\x1b[34mabc\x1b[0m", "\x1b[34mdef\x1b[0m"]);
    }

    #[test]
    fn keycap_wrapping_respects_the_complete_line_width() {
        let text = format!("{}-end.txt", "1\u{fe0f}\u{20e3}".repeat(30));
        let lines = wrap_line(&text, 39).unwrap();
        assert_eq!(lines.concat(), text);
        assert!(lines.iter().all(|line| measure_text_width(line) <= 39));
    }

    #[test]
    fn wrapping_preserves_grapheme_boundaries_even_across_ansi() {
        for text in [
            "1\u{fe0f}\u{20e3}".repeat(9),
            "\u{1f469}\u{200d}\u{1f4bb}".repeat(9),
            "\u{1f1ef}\u{1f1f5}".repeat(9),
            "\u{1f44d}\u{1f3fd}".repeat(9),
            "e\u{301}日本abc".repeat(9),
            "\u{301}e\u{301}".to_owned(),
            "1\x1b[34m\u{fe0f}\u{20e3}\x1b[0m".repeat(9),
            String::new(),
        ] {
            let plain = console::strip_ansi_codes(&text);
            let mut boundaries: Vec<_> = plain.grapheme_indices(true).map(|(i, _)| i).collect();
            boundaries.push(plain.len());
            for width in [2, 3, 4, 7, 39] {
                let lines = wrap_line(&text, width).unwrap();
                let mut joined = String::new();
                for line in lines {
                    let visible = console::strip_ansi_codes(&line);
                    assert!(measure_text_width(&visible) <= width, "{line:?}");
                    joined.push_str(&visible);
                    assert!(boundaries.contains(&joined.len()), "{text:?}, {joined:?}");
                }
                assert_eq!(joined, plain);
            }
        }
    }

    #[test]
    fn oversized_cluster_is_an_explicit_layout_error() {
        assert!(wrap_line("1\u{fe0f}\u{20e3}", 1).is_err());
        let text = "\u{17a0}\u{17d2}\u{17ab}";
        let cluster_width = measure_text_width(text);
        assert_eq!(text.graphemes(true).count(), 1);
        assert!(wrap_line(text, cluster_width - 1).is_err());
        assert_eq!(wrap_line(text, cluster_width).unwrap(), [text]);
        let wide_cluster = "\u{17d8}";
        assert_eq!(measure_text_width(wide_cluster), 3);
        let mut doc = document(wide_cluster);
        assert!(doc.page((200, 3)).is_err());
        doc.text = "body".to_owned();
        doc.root = wide_cluster.to_owned();
        assert!(doc.page((200, 3)).is_err());
        for width in [3, 10] {
            assert!(wrap_line(&format!("a{}", "\u{09cb}".repeat(10)), width).is_err());
        }
    }

    #[test]
    fn labels_use_visible_content_not_only_the_top_row() {
        let mut page = document(&"row\n".repeat(60)).page((22, 100)).unwrap();
        page.sections = [4..54, 55..59];
        page.height = 2;
        assert_eq!(page.section_label(0), "PLAN");
        assert_eq!(page.section_label(2), "PLAN");
        assert_eq!(page.section_label(3), "KEEP");
        assert_eq!(page.section_label(53), "KEEP");
        assert_eq!(page.section_label(54), "DELETE");
        assert_eq!(page.section_label(55), "DELETE");
        assert_eq!(page.section_label(59), "PLAN");
        page.height = 16;
        assert_eq!(page.section_label(44), "KEEP + DELETE");
        page.height = 3;
        assert_eq!(page.section_label(53), "KEEP + DELETE");
    }

    #[test]
    fn status_colors_only_section_names_and_preserves_plain_text_and_width() {
        let mut page = document(&"row\n".repeat(60)).page((22, 100)).unwrap();
        page.sections = [4..54, 55..59];
        for (top, height, expected) in [
            (0, 2, "\x1b[36m\x1b[1mPLAN\x1b[0m"),
            (4, 2, "\x1b[32m\x1b[1mKEEP\x1b[0m"),
            (55, 2, "\x1b[31m\x1b[1mDELETE\x1b[0m"),
            (
                44,
                16,
                "\x1b[32m\x1b[1mKEEP\x1b[0m + \x1b[31m\x1b[1mDELETE\x1b[0m",
            ),
        ] {
            page.height = height;
            page.width = 100;
            let colored = page.status_line(top, true);
            let plain = page.status_line(top, false);
            assert!(colored.starts_with(expected), "{colored:?}");
            assert_eq!(console::strip_ansi_codes(&colored), plain);
            assert!(!plain.contains('\x1b'));
            assert!(colored.ends_with(&format!(
                " | lines {}-{} / 60",
                top + 1,
                (top + height).min(60)
            )));
            for width in 2..40 {
                page.width = width;
                let colored = page.status_line(top, true);
                assert!(measure_text_width(&colored) <= width);
                assert_eq!(
                    console::strip_ansi_codes(&colored),
                    page.status_line(top, false)
                );
                if colored.contains('\x1b') {
                    // A clipped label must not leave styling active for the controls.
                    let last_ansi = AnsiCodeIterator::new(&colored)
                        .filter(|(_, ansi)| *ansi)
                        .map(|(part, _)| part)
                        .last();
                    assert_eq!(last_ansi, Some("\x1b[0m"));
                }
            }
        }
    }

    #[test]
    fn section_navigation_cycles_reachable_targets_in_both_directions() {
        let mut page = document(&"row\n".repeat(60)).page((22, 100)).unwrap();
        for sections in [[4..54, 55..60], [4..6, 7..60], [4..30, 31..60]] {
            page.sections = sections;
            let last = page.lines.len() - page.height;
            let targets = page.sections.each_ref().map(|range| range.start.min(last));
            let mut position = Position { top: targets[0] };
            for _ in 0..3 {
                position.key(Key::Tab, &page);
                assert_eq!(position.top, targets[1]);
                position.key(Key::Tab, &page);
                assert_eq!(position.top, targets[0]);
                position.key(Key::BackTab, &page);
                assert_eq!(position.top, targets[1]);
                position.key(Key::BackTab, &page);
                assert_eq!(position.top, targets[0]);
            }
        }
        page.height = 100;
        let mut position = Position::default();
        for key in [Key::Tab, Key::BackTab, Key::Tab] {
            position.key(key, &page);
            assert_eq!(position.top, 0);
        }
    }

    #[test]
    fn section_ranges_reflow_with_wrapped_headings() {
        let mut doc = document(
            "intro\nkeep heading heading\nitem\n\ndelete heading heading\nitem\nwarning\n",
        );
        doc.sections = [1..3, 4..6];
        for size in [(30, 12), (30, 40)] {
            let page = doc.page(size).unwrap();
            for (source, rendered) in doc.sections.iter().zip(&page.sections) {
                assert_eq!(page.source_lines[rendered.start], source.start);
                assert_eq!(page.source_lines[rendered.end - 1], source.end - 1);
            }
        }
    }

    #[test]
    fn tab_cycles_when_delete_section_starts_inside_the_last_page() {
        let doc = document(&"row\n".repeat(60));
        let mut page = doc.page((22, 100)).unwrap();
        page.sections = [4..54, 55..60];
        let mut position = Position { top: 4 };
        position.key(Key::Tab, &page);
        assert_eq!(position.top, 44);
        position.key(Key::Tab, &page);
        assert_eq!(position.top, 4);
        position.key(Key::BackTab, &page);
        assert_eq!(position.top, 44);
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
