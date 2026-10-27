//! Pop-up menus in the classic Windows style Winamp used: grey face,
//! navy highlight, check marks, separators and sub-menus. Drawn here so
//! they look the same on every system.

use crate::font;
use crate::frame::{Frame, argb};

const FACE: [u8; 3] = [212, 208, 200];
const HI: [u8; 3] = [255, 255, 255];
const SHADOW: [u8; 3] = [128, 128, 128];
const DARK: [u8; 3] = [64, 64, 64];
const TEXT: [u8; 3] = [0, 0, 0];
const GREY_TEXT: [u8; 3] = [128, 128, 128];
const SEL: [u8; 3] = [10, 36, 106];
const SEL_TEXT: [u8; 3] = [255, 255, 255];
const PX: f32 = 11.0;
const ROW: i32 = 18;
const SEP: i32 = 8;

#[derive(Clone, Debug)]
pub enum Item {
    Action { id: String, label: String, checked: bool, enabled: bool, shortcut: String },
    Submenu { label: String, items: Vec<Item> },
    Separator,
}

impl Item {
    pub fn action(id: impl Into<String>, label: impl Into<String>) -> Item {
        Item::Action { id: id.into(), label: label.into(), checked: false, enabled: true, shortcut: String::new() }
    }

    pub fn checked(mut self, on: bool) -> Item {
        if let Item::Action { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }

    pub fn enabled(mut self, on: bool) -> Item {
        if let Item::Action { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    pub fn shortcut(mut self, s: &str) -> Item {
        if let Item::Action { shortcut, .. } = &mut self {
            *shortcut = s.to_owned();
        }
        self
    }

    pub fn submenu(label: impl Into<String>, items: Vec<Item>) -> Item {
        Item::Submenu { label: label.into(), items }
    }

    fn height(&self) -> i32 {
        if matches!(self, Item::Separator) { SEP } else { ROW }
    }

    /// The label as shown, in the language in use.
    fn label(&self) -> &str {
        match self {
            Item::Action { label, .. } | Item::Submenu { label, .. } => crate::lang::t(label),
            Item::Separator => "",
        }
    }
}

/// The size a menu needs.
pub fn size(items: &[Item]) -> (u32, u32) {
    let label_w = items.iter().map(|i| font::measure(i.label(), PX, false)).max().unwrap_or(0);
    let short_w = items
        .iter()
        .map(|i| match i {
            Item::Action { shortcut, .. } if !shortcut.is_empty() => font::measure(shortcut, PX, false) + 24,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    let h: i32 = items.iter().map(Item::height).sum();
    ((label_w + short_w + 22 + 24).max(120) as u32, (h + 6) as u32)
}

/// The item index at a point, if it's a selectable row.
pub fn hit(items: &[Item], x: i32, y: i32, width: i32) -> Option<usize> {
    if x < 2 || x >= width - 2 {
        return None;
    }
    let mut top = 3;
    for (i, item) in items.iter().enumerate() {
        let h = item.height();
        if y >= top && y < top + h {
            return (!matches!(item, Item::Separator)).then_some(i);
        }
        top += h;
    }
    None
}

/// The top of item `index` (for placing its sub-menu).
pub fn item_top(items: &[Item], index: usize) -> i32 {
    3 + items.iter().take(index).map(Item::height).sum::<i32>()
}

pub fn draw(frame: &mut Frame, items: &[Item], hover: Option<usize>) {
    let (w, h) = size(items);
    frame.resize(w, h);
    let (w, h) = (w as i32, h as i32);
    frame.fill(0, 0, w, h, argb(FACE));
    // The raised Windows 98 border.
    frame.fill(0, 0, w, 1, argb(FACE));
    frame.fill(0, 0, 1, h, argb(FACE));
    frame.fill(1, 1, w - 2, 1, argb(HI));
    frame.fill(1, 1, 1, h - 2, argb(HI));
    frame.fill(1, h - 2, w - 2, 1, argb(SHADOW));
    frame.fill(w - 2, 1, 1, h - 2, argb(SHADOW));
    frame.fill(0, h - 1, w, 1, argb(DARK));
    frame.fill(w - 1, 0, 1, h, argb(DARK));
    let mut top = 3;
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Separator => {
                frame.fill(4, top + 3, w - 8, 1, argb(SHADOW));
                frame.fill(4, top + 4, w - 8, 1, argb(HI));
                top += SEP;
                continue;
            }
            Item::Action { label, checked, enabled, shortcut, .. } => {
                let on = hover == Some(i) && *enabled;
                if on {
                    frame.fill(3, top, w - 6, ROW, argb(SEL));
                }
                let col = if !enabled { GREY_TEXT } else if on { SEL_TEXT } else { TEXT };
                if *checked {
                    for (dx, dy) in [(0, 2), (1, 3), (2, 4), (3, 3), (4, 2), (5, 1), (6, 0)] {
                        frame.fill(8 + dx, top + 6 + dy, 1, 2, argb(col));
                    }
                }
                font::draw(frame, 22, top + 13, crate::lang::t(label), PX, col, w - 30, false);
                if !shortcut.is_empty() {
                    let sw = font::measure(shortcut, PX, false);
                    font::draw(frame, w - 12 - sw, top + 13, shortcut, PX, col, sw + 2, false);
                }
            }
            Item::Submenu { label, .. } => {
                let on = hover == Some(i);
                if on {
                    frame.fill(3, top, w - 6, ROW, argb(SEL));
                }
                let col = if on { SEL_TEXT } else { TEXT };
                font::draw(frame, 22, top + 13, crate::lang::t(label), PX, col, w - 40, false);
                for d in 0..4 {
                    frame.fill(w - 12 + d, top + 5 + d, 1, 8 - 2 * d, argb(col));
                }
            }
        }
        top += ROW;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_size() {
        let items = vec![Item::action("a", "Play file…").shortcut("L"), Item::Separator, Item::submenu("Skins", vec![]), Item::action("q", "Exit").checked(true)];
        let (w, h) = size(&items);
        assert!(w >= 120);
        assert_eq!(h as i32, ROW * 3 + SEP + 6);
        assert_eq!(hit(&items, 30, 4, w as i32), Some(0));
        assert_eq!(hit(&items, 30, 3 + ROW + 2, w as i32), None);
        assert_eq!(hit(&items, 30, 3 + ROW + SEP + 2, w as i32), Some(2));
        let mut f = Frame::new(1, 1);
        draw(&mut f, &items, Some(0));
        if let Ok(dir) = std::env::var("GAMP_SNAPSHOTS") {
            f.to_image().save(format!("{dir}/menu.png")).unwrap();
        }
    }
}
