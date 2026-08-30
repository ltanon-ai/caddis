//! worker_board_frame.rs — CARD-0226. Adaptive frame renderer.
//! Width comes from the terminal (Windows console info), override
//! CADDIS_BOARD_WIDTH, default 80. Every row is padded to the EXACT
//! visible width so borders never break when the screen resizes.
//! ANSI colors are zero-width: visible_len strips escape sequences.

pub fn width() -> usize {
    if let Some(w) = std::env::var_os("CADDIS_BOARD_WIDTH") {
        // swallow: fail-safe-by-law
        if let Ok(n) = w.to_string_lossy().trim().parse::<usize>() {
            if (20..=400).contains(&n) {
                return n;
            }
        }
    }
    terminal_width().unwrap_or(80).max(40)
}

#[cfg(windows)]
fn terminal_width() -> Option<usize> {
    #[repr(C)]
    struct Csbi {
        size_x: i16,
        size_y: i16,
        cur_x: i16,
        cur_y: i16,
        attr: u16,
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
        max_x: i16,
        max_y: i16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> *mut core::ffi::c_void;
        fn GetConsoleScreenBufferInfo(h: *mut core::ffi::c_void, b: *mut Csbi) -> i32;
    }
    unsafe {
        let h = GetStdHandle(-11i32 as u32);
        if h.is_null() {
            return None;
        }
        let mut b: Csbi = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(h, &mut b) == 0 {
            return None;
        }
        Some((b.right - b.left + 1).max(1) as usize)
    }
}

#[cfg(not(windows))]
fn terminal_width() -> Option<usize> {
    None
}

pub struct Frame {
    pub w: usize,
    out: String,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            w: width(),
            out: String::new(),
        }
    }

    pub fn header(&mut self, title: &str) {
        let fill = self.w.saturating_sub(title.chars().count() + 2);
        self.out.push_str(&format!(
            "{BOLD}{CYAN}● {title} {DIM}┄{}┄{RESET}\n",
            "─".repeat(fill.saturating_sub(2))
        ));
    }

    /// The golden summary strip: one glance, everything that matters.
    pub fn strip(&mut self, left: &str, right: &str) {
        let lw = visible_len(left);
        let rw = visible_len(right);
        let gap = self.w.saturating_sub(lw + rw + 4).max(1);
        self.out.push_str(&format!(
            "{BOLD}{CYAN}▐ {RESET}{BOLD}{TEXT}{left}{RESET}{DIM}{}{RESET}{TEXT}{right}{RESET}{BOLD}{CYAN}▌{RESET}\n",
            " ".repeat(gap),
        ));
    }

    pub fn section(&mut self, icon: &str, name: &str) {
        let fill = self
            .w
            .saturating_sub(name.chars().count() + icon.chars().count() + 5);
        self.out.push_str(&format!(
            "{BOLD}{CYAN}{icon} {name} {DIM}┄{}{RESET}\n",
            "┄".repeat(fill)
        ));
    }

    /// One labeled row; the value is truncated (…) to fit the width.
    pub fn row(&mut self, label: &str, value: &str, color: &str) {
        let label_w = 10;
        let avail = self.w.saturating_sub(label_w + 5).max(4);
        let v = truncate_visible(value, avail);
        let pad = avail.saturating_sub(visible_len(&v));
        self.out.push_str(&format!(
            "{DIM}│{RESET} {color}{label:<label_w$}{RESET}{DIM}│{RESET} {v}{}{DIM}│{RESET}\n",
            " ".repeat(pad),
        ));
    }

    /// A proportional bar row: [####-----] pct.
    pub fn bar(&mut self, label: &str, pct: u64, suffix: &str) {
        let bar_w = (self.w / 4).clamp(8, 40) as u64;
        let filled = if pct > 100 { bar_w } else { pct * bar_w / 100 };
        let bar: String = "█".repeat(filled as usize) + &"░".repeat((bar_w - filled) as usize);
        let color = if pct >= 90 {
            RED
        } else if pct >= 70 {
            YELLOW
        } else {
            GREEN
        };
        self.row(label, &format!("{bar} {pct:3}% {suffix}"), color);
    }

    pub fn finish(self) -> String {
        self.out
    }
}

pub fn truncate_visible(s: &str, max: usize) -> String {
    if visible_len(s) <= max {
        return paint_plain(s);
    }
    // strip codes, then hard-cut with an ellipsis
    let plain: String = strip_ansi(s);
    let mut out = String::new();
    for (i, ch) in plain.chars().enumerate() {
        if i + 1 >= max.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn paint_plain(s: &str) -> String {
    s.to_string()
}

pub fn visible_len(s: &str) -> usize {
    strip_ansi(s).chars().count()
}
// Estate design DNA (Pelėda/Showr palette — idea ledger, provenance kept):
// warm near-black bg, signature gold accent, off-white text, steel lines,
// LED green/red statuses. Truecolor; terminals without it degrade fine.
pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[38;2;208;80;80m";
pub const GREEN: &str = "\x1b[38;2;74;159;90m";
pub const YELLOW: &str = "\x1b[38;2;224;160;80m";
pub const CYAN: &str = "\x1b[38;2;201;160;74m";
pub const TEXT: &str = "\x1b[38;2;208;206;200m";

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}
