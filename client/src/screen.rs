use crossterm::{
    cursor,
    style::{self, Stylize},
    terminal, QueueableCommand,
};
use log::trace;
use std::io::{Stdout, Write};

use crate::{line::*, util::styles::*};

const SPACE: char = ' ';
const VBAR: char = '│';
const HBAR: char = '—';

struct Bits {
    stat_begin: String,
    stat_begin_chars: usize,
    stat_end: String,
    stat_end_chars: usize,
    full_hline: String,
}

impl Bits {
    fn new(width: u16) -> Bits {
        let mut start = Line::default();
        let mut end = Line::default();
        start.pushf(String::from(VBAR), &DIM);
        start.push(" ");
        end.push(" ");
        end.pushf(String::from(VBAR), &DIM);

        let mut hline = Line::default();
        {
            let mut s = String::with_capacity(width as usize);
            for _ in 0..width {
                s.push(HBAR);
            }
            hline.pushf(&s, &DIM);
        }

        let start_len = start.len();
        let end_len = end.len();

        Bits {
            stat_begin: start.first_n_chars(start_len).to_string(),
            stat_end: end.first_n_chars(end_len).to_string(),
            stat_begin_chars: start_len,
            stat_end_chars: end_len,
            full_hline: hline.first_n_chars((width + 1) as usize).to_string(),
        }
    }
}

pub struct Screen {
    lines: Vec<Line>,
    input: Vec<char>,
    input_ip: u16,
    roster: Vec<Line>,
    roster_width: u16,
    stat_ul: Line,
    stat_ur: Line,
    stat_ll: Line,
    lines_dirty: bool,
    input_dirty: bool,
    roster_dirty: bool,
    stat_dirty: bool,
    bits: Bits,

    lines_scroll: u16,
    roster_scroll: u16,
    last_x_size: u16,
    last_y_size: u16,
}

impl Screen {
    pub fn new(term: &mut Stdout, roster_chars: u16) -> crossterm::Result<Screen> {
        terminal::enable_raw_mode()?;
        let (x, y): (u16, u16) = terminal::size()?;
        term.queue(cursor::Hide)?.queue(terminal::DisableLineWrap)?;
        term.queue(terminal::SetTitle("Fresh Client"))?;
        term.flush()?;

        Ok(Screen {
            lines: Vec::new(),
            input: Vec::new(),
            roster: Vec::new(),
            roster_width: roster_chars,
            input_ip: 0,
            stat_ul: Line::default(),
            stat_ur: Line::default(),
            stat_ll: Line::default(),
            lines_dirty: true,
            input_dirty: true,
            roster_dirty: true,
            stat_dirty: true,
            lines_scroll: 0,
            roster_scroll: 0,
            last_x_size: x,
            last_y_size: y,
            bits: Bits::new(x),
        })
    }

    /// Return the height of the main scrollback window.
    pub fn get_main_height(&self) -> u16 {
        self.last_y_size - 2
    }

    /// Return the number of `Line`s in the scrollback buffer.
    pub fn get_scrollback_length(&self) -> usize {
        self.lines.len()
    }

    /// Trim the scrollback buffer to the latest `n` lines.
    pub fn prune_scrollback(&mut self, n: usize) {
        if n >= self.lines.len() {
            return;
        }
        let new_zero = self.lines.len() - n;

        self.lines.drain(0..new_zero);
        self.lines_dirty = true;
    }

    /// Push the supplied line onto the end of the scrollback buffer.
    pub fn push_line(&mut self, line: Line) {
        self.lines.push(line);
        self.lines_dirty = true;
    }

    /// Populate the roster with the given slice of strings.
    pub fn set_roster<T: AsRef<str>>(&mut self, items: &[T]) {
        self.roster = items
            .iter()
            .map(|s| {
                let mut l = Line::default();
                l.push(s.as_ref());
                l
            })
            .collect();

        self.roster_dirty = true;
    }
    /// Get number of characters in the input line
    pub fn get_input_length(&self) -> usize {
        self.input.len()
    }

    /// Add a `char` to the input line.
    pub fn input_char(&mut self, ch: char) {
        let input_ip = self.input_ip as usize;

        if input_ip >= self.input.len() {
            self.input.push(ch);
        } else {
            self.input.insert(input_ip, ch);
        }

        self.input_ip = (input_ip + 1) as u16;
        self.input_dirty = true;
    }

    pub fn input_backspace(&mut self) {
        let ilen = self.input.len();
        let input_ip = self.input_ip as usize;

        if ilen == 0 || input_ip == 0 {
            return;
        }

        self.input_ip = (input_ip - 1) as u16;
        self.input.remove(input_ip - 1);
        self.input_dirty = true;
    }

    pub fn input_delete_words(&mut self, words_to_delete: i32) {
        let uip = self.input_ip as usize;
        let ilen = self.input.len();

        if (uip == ilen && words_to_delete > 0) || (uip == 0 && words_to_delete < 0) {
            return;
        }

        self.input_dirty = true;

        if words_to_delete > 0 {
            let mut i = uip;
            while i < ilen && self.input[i].is_whitespace() {
                i += 1;
            }
            if i >= ilen {
                return;
            }

            let mut j = i;
            for _ in 0..words_to_delete {
                while j < ilen && !self.input[j].is_whitespace() {
                    j += 1;
                }
                while j < ilen && self.input[j].is_whitespace() {
                    j += 1;
                }
            }

            self.input.drain(i..j);
        } else {
            let words_to_delete_abs = words_to_delete.abs();
            let mut i = uip;

            for _ in 0..words_to_delete_abs {
                while i > 0 && self.input[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !self.input[i - 1].is_whitespace() {
                    i -= 1;
                }
            }

            self.input.drain(i..uip);
            self.input_ip = i as u16;
        }
    }

    /// Move the input cursor by `n_chars` characters. Negative values move the
    /// cursor to the left.
    pub fn input_skip_chars(&mut self, n_chars: i16) {
        let cur = self.input_ip as i16;
        let new = cur + n_chars;
        let ilen = self.input.len() as u16;

        self.input_ip = if new < 0 {
            0
        } else {
            let new = new as u16;
            if new > ilen {
                ilen
            } else {
                new
            }
        };

        self.input_dirty = true;
    }

    pub fn input_skip_words(&mut self, words_to_skip: i32) {
        let uip = self.input_ip as usize;

        if uip == self.input.len() && words_to_skip > 0 {
            return;
        }

        if uip == 0 && words_to_skip < 0 {
            return;
        }

        self.input_dirty = true;
        let mut words_skipped = 0;

        if words_to_skip > 0 {
            let mut in_ws = self.input[uip].is_whitespace();

            for (i, c) in self.input[uip..].iter().enumerate() {
                if in_ws {
                    if !c.is_whitespace() {
                        in_ws = false;
                    }
                } else if c.is_whitespace() {
                    words_skipped += 1;
                    if words_skipped >= words_to_skip {
                        self.input_ip = (uip + i) as u16;
                        return;
                    }
                    in_ws = true;
                }
            }
            self.input_ip = self.input.len() as u16;
        } else {
            let words_to_skip_abs = words_to_skip.abs();
            let mut in_ws = false;
            if uip < self.input.len() && self.input[uip - 1].is_whitespace() {
                in_ws = true;
            }

            for (i, c) in self.input[..uip].iter().rev().enumerate() {
                if in_ws {
                    if !c.is_whitespace() {
                        in_ws = false;
                    }
                } else if c.is_whitespace() {
                    words_skipped += 1;
                    if words_skipped >= words_to_skip_abs {
                        self.input_ip = (uip - i) as u16;
                        return;
                    }
                    in_ws = true;
                }
            }
            self.input_ip = 0;
        }
    }

    /// Scroll the input line by `n_chars` characters. Negative values scroll the
    /// line down.
    pub fn scroll_lines(&mut self, n_chars: i16) {
        let cur = self.lines_scroll as i16;
        let new = (cur + n_chars).max(0);
        self.lines_scroll = new as u16;
        self.lines_dirty = true;
    }

    /// Return the contents of the input line as a String and clear the input line.
    pub fn pop_input(&mut self) -> Vec<char> {
        let new_v = std::mem::take(&mut self.input);
        self.input_ip = 0;
        self.input_dirty = true;
        new_v
    }

    pub fn set_stat_ll(&mut self, new_stat: Line) {
        self.stat_ll = new_stat;
        self.stat_dirty = true;
    }
    pub fn set_stat_ul(&mut self, new_stat: Line) {
        self.stat_ul = new_stat;
        self.stat_dirty = true;
    }
    pub fn set_stat_ur(&mut self, new_stat: Line) {
        self.stat_ur = new_stat;
        self.stat_dirty = true;
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols != self.last_x_size || rows != self.last_y_size {
            if cols != self.last_x_size {
                let horizontal_line =
                    String::from_iter(std::iter::repeat(HBAR).take(cols as usize));
                let mut line = Line::default();
                line.pushf(&horizontal_line, &DIM);
                self.bits.full_hline = line.first_n_chars(cols as usize).to_string();
            }

            self.lines_dirty = true;
            self.input_dirty = true;
            self.roster_dirty = true;
            self.stat_dirty = true;
            self.last_x_size = cols;
            self.last_y_size = rows;
        }
    }

    fn refresh_lines(
        &mut self,
        term: &mut Stdout,
        width: u16,
        height: u16,
    ) -> crossterm::Result<()> {
        trace!("Screen::refresh_lines(..., {}, {}) called", &width, &height);
        let blank: String = {
            let mut s = String::default();
            for _ in 0..width {
                s.push(SPACE);
            }
            s
        };
        let mut y = height - 1;
        let width = width as usize;
        let mut count_back: u16 = 0;
        for aline in self.lines.iter_mut().rev() {
            for row in aline.lines(width).iter().rev() {
                if y == 0 {
                    break;
                }
                if count_back >= self.lines_scroll {
                    term.queue(cursor::MoveTo(0, y))?
                        .queue(style::Print(&blank))?
                        .queue(cursor::MoveToColumn(0))?
                        .queue(style::Print(&row))?;
                    y -= 1;
                }
                count_back += 1;
            }
            if y == 0 {
                break;
            }
        }

        if y > 1 && self.lines_scroll > 0 {
            let adjust: i16 = (y - 1) as i16;
            self.scroll_lines(-adjust);
        } else {
            while y > 0 {
                term.queue(cursor::MoveTo(0, y))?
                    .queue(style::Print(&blank))?;
                y -= 1;
            }
            self.lines_dirty = false;
        }
        Ok(())
    }

    fn refresh_roster(
        &mut self,
        term: &mut Stdout,
        x_start: u16,
        height: u16,
    ) -> crossterm::Result<()> {
        trace!(
            "Screen::refresh_roster(..., {}, {}) called",
            &x_start,
            &height
        );

        let roster_width_with_vbar: usize = (self.roster_width as usize) + 1;
        let roster_width: usize = self.roster_width as usize;

        let blank_line: String = {
            let mut s = String::default();
            for _ in 0..self.roster_width {
                s.push(SPACE);
            }
            let mut l = Line::default();
            l.pushf(String::from(VBAR), &DIM);
            l.push(&s);
            l.first_n_chars(roster_width_with_vbar).to_string()
        };

        let mut y: u16 = 1;
        let target_y = height;
        let roster_scroll = self.roster_scroll as usize;

        for (i, line) in self.roster.iter_mut().enumerate() {
            if y == target_y {
                break;
            }
            if i >= roster_scroll {
                term.queue(cursor::MoveTo(x_start, y))?
                    .queue(style::Print(&blank_line))?
                    .queue(cursor::MoveTo(x_start + 1, y))?
                    .queue(style::Print(line.first_n_chars(roster_width)))?;
                y += 1;
            }
        }

        while y < height {
            term.queue(cursor::MoveTo(x_start, y))?
                .queue(style::Print(&blank_line))?;
            y += 1;
        }

        self.roster_dirty = false;
        Ok(())
    }

    /// Refresh the screen, if necessary.
    fn refresh_input(&mut self, term: &mut Stdout) -> crossterm::Result<()> {
        term.queue(cursor::MoveTo(0, self.last_y_size - 1))?
            .queue(terminal::Clear(terminal::ClearType::CurrentLine))?
            .queue(cursor::MoveToColumn(0))?;

        let third = self.last_x_size / 3;
        let maxpos = self.last_x_size - third;
        let startpos = if self.input.len() < self.last_x_size as usize || self.input_ip < third {
            0
        } else if self.input_ip > maxpos {
            self.input_ip - maxpos
        } else {
            self.input_ip - third
        };

        let input_ip_us = self.input_ip as usize;
        let endpos = ((startpos + self.last_x_size) as usize).min(self.input.len());

        for (i, c) in self.input[startpos as usize..endpos].iter().enumerate() {
            let i = i + startpos as usize;
            let c = if i == input_ip_us {
                style::style(*c).attribute(style::Attribute::Reverse)
            } else {
                style::style(*c)
            };
            term.queue(style::PrintStyledContent(c))?;
        }

        if input_ip_us == self.input.len() {
            let cch = style::style(' ').attribute(style::Attribute::Reverse);
            term.queue(style::PrintStyledContent(cch))?;
        }

        self.input_dirty = false;
        Ok(())
    }

    fn refresh_stat(&mut self, term: &mut Stdout) -> crossterm::Result<()> {
        trace!("Screen::refresh_stat(...) called");

        let stat_padding = 2 + self.bits.stat_begin_chars + self.bits.stat_end_chars;
        let stat_width = (self.last_x_size as usize) - stat_padding;
        let lower_line_y = self.last_y_size - 2;

        term.queue(cursor::MoveTo(0, lower_line_y))?
            .queue(style::Print(&self.bits.full_hline))?
            .queue(cursor::MoveTo(1, lower_line_y))?
            .queue(style::Print(&self.bits.stat_begin))?
            .queue(style::Print(self.stat_ll.first_n_chars(stat_width)))?
            .queue(style::Print(&self.bits.stat_end))?;

        let total_space = self.last_x_size
            - (3 + (self.bits.stat_begin_chars * 2) + (self.bits.stat_end_chars * 2)) as u16;
        let space_per_section: usize = (total_space / 2) as usize;
        let abbreviation_space = space_per_section - 3;

        term.queue(cursor::MoveTo(0, 0))?
            .queue(style::Print(&self.bits.full_hline))?
            .queue(cursor::MoveTo(1, 0))?
            .queue(style::Print(&self.bits.stat_begin))?;
        if self.stat_ul.len() > space_per_section {
            term.queue(style::Print(self.stat_ul.first_n_chars(abbreviation_space)))?
                .queue(style::Print("..."))?;
        } else {
            term.queue(style::Print(self.stat_ul.first_n_chars(space_per_section)))?;
        }
        term.queue(style::Print(&self.bits.stat_end))?;

        let upper_right_offset: u16 = if self.stat_ur.len() > space_per_section {
            self.last_x_size
                - (2 + self.bits.stat_begin_chars + self.bits.stat_end_chars + space_per_section)
                    as u16
        } else {
            self.last_x_size
                - (2 + self.bits.stat_begin_chars + self.bits.stat_end_chars + self.stat_ur.len())
                    as u16
        };

        term.queue(cursor::MoveTo(upper_right_offset, 0))?
            .queue(style::Print(&self.bits.stat_begin))?;
        if self.stat_ur.len() > space_per_section {
            term.queue(style::Print(self.stat_ur.first_n_chars(abbreviation_space)))?
                .queue(style::Print("..."))?;
        } else {
            term.queue(style::Print(self.stat_ur.first_n_chars(space_per_section)))?;
        }
        term.queue(style::Print(&self.bits.stat_end))?;

        self.stat_dirty = false;
        Ok(())
    }

    fn announce_term_too_small(&self, term: &mut Stdout) -> crossterm::Result<()> {
        term.queue(terminal::Clear(terminal::ClearType::All))?
            .queue(cursor::MoveTo(0, 0))?
            .queue(style::Print("Terminal window is too small!"))?;
        term.flush()?;

        Ok(())
    }

    pub fn refresh(&mut self, term: &mut Stdout) -> Result<(), String> {
        trace!("Screen::refresh(...) called");

        if !(self.lines_dirty || self.input_dirty || self.roster_dirty || self.stat_dirty) {
            return Ok(());
        }

        let roster_width = self.roster_width + 1;
        let main_width = self.last_x_size - roster_width;
        let main_height = self.last_y_size - 2;

        if main_width < 20 || main_height < 5 {
            return self
                .announce_term_too_small(term)
                .map_err(|e| format!("{}", e));
        }

        if self.input_dirty {
            self.refresh_input(term).map_err(|e| format!("{}", e))?;
        }
        if self.lines_dirty {
            self.refresh_lines(term, main_width, main_height)
                .map_err(|e| format!("{}", e))?;
        }
        if self.roster_dirty {
            self.refresh_roster(term, main_width, main_height)
                .map_err(|e| format!("{}", e))?;
        }
        if self.stat_dirty {
            self.refresh_stat(term).map_err(|e| format!("{}", e))?;
        }

        term.flush().map_err(|e| format!("{}", e))?;

        Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let mut term = std::io::stdout();
        term.queue(cursor::Show)
            .unwrap()
            .queue(terminal::EnableLineWrap)
            .unwrap()
            .queue(terminal::Clear(terminal::ClearType::All))
            .unwrap()
            .queue(cursor::MoveTo(0, 0))
            .unwrap();
        term.flush().unwrap();
        terminal::disable_raw_mode().unwrap();
    }
}
