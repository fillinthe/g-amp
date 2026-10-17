//! Winamp's docking: windows that touch move together with the main
//! window, and a dragged window snaps to other windows and the screen
//! edges within 10 pixels.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    fn overlaps_x(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right()
    }
    fn overlaps_y(&self, o: &Rect) -> bool {
        self.y < o.bottom() && o.y < self.bottom()
    }
}

/// Whether two windows share an edge.
pub fn touching(a: &Rect, b: &Rect) -> bool {
    let near = |p: i32, q: i32| (p - q).abs() <= 1;
    ((near(a.right(), b.x) || near(b.right(), a.x)) && a.overlaps_y(b)) || ((near(a.bottom(), b.y) || near(b.bottom(), a.y)) && a.overlaps_x(b))
}

/// Everything docked to `start`, directly or through others (indices).
pub fn docked_group(rects: &[Rect], start: usize) -> Vec<usize> {
    let mut group = vec![start];
    let mut i = 0;
    while i < group.len() {
        let g = group[i];
        for (j, r) in rects.iter().enumerate() {
            if !group.contains(&j) && touching(&rects[g], r) {
                group.push(j);
            }
        }
        i += 1;
    }
    group
}

/// The nudge (dx, dy) that snaps `moving` to `others` and `screen`,
/// or 0 on an axis with nothing within `threshold`.
pub fn snap(moving: &[Rect], others: &[Rect], screen: Option<Rect>, threshold: i32) -> (i32, i32) {
    let mut best_x: Option<i32> = None;
    let mut best_y: Option<i32> = None;
    let consider = |best: &mut Option<i32>, d: i32| {
        if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
            *best = Some(d);
        }
    };
    for m in moving {
        for o in others {
            if m.overlaps_y(o) || (m.y - o.bottom()).abs() <= threshold || (o.y - m.bottom()).abs() <= threshold {
                for d in [o.x - m.right(), o.right() - m.x, o.x - m.x, o.right() - m.right()] {
                    consider(&mut best_x, d);
                }
            }
            if m.overlaps_x(o) || (m.x - o.right()).abs() <= threshold || (o.x - m.right()).abs() <= threshold {
                for d in [o.y - m.bottom(), o.bottom() - m.y, o.y - m.y, o.bottom() - m.bottom()] {
                    consider(&mut best_y, d);
                }
            }
        }
        if let Some(s) = screen {
            for d in [s.x - m.x, s.right() - m.right()] {
                consider(&mut best_x, d);
            }
            for d in [s.y - m.y, s.bottom() - m.bottom()] {
                consider(&mut best_y, d);
            }
        }
    }
    (best_x.unwrap_or(0), best_y.unwrap_or(0))
}

/// Where a window `w` wide docks beside `group`: on its right, or on its
/// left when the right would run off `screen` and the left fits.
pub fn beside(group: &[Rect], w: i32, screen: Option<Rect>) -> i32 {
    let right = group.iter().map(Rect::right).max().unwrap_or(0);
    let left = group.iter().map(|r| r.x).min().unwrap_or(0) - w;
    match screen {
        Some(s) if right + w > s.right() && left >= s.x => left,
        _ => right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect { x: 100, y: 100, w: 275, h: 116 };

    #[test]
    fn stacked_windows_are_one_group() {
        let eq = Rect { x: 100, y: 216, w: 275, h: 116 };
        let pl = Rect { x: 100, y: 332, w: 275, h: 116 };
        let away = Rect { x: 600, y: 100, w: 275, h: 116 };
        let g = docked_group(&[MAIN, eq, pl, away], 0);
        assert_eq!(g, vec![0, 1, 2]);
    }

    #[test]
    fn docks_on_the_side_that_fits() {
        let s = Rect { x: 0, y: 0, w: 1440, h: 900 };
        assert_eq!(beside(&[MAIN], 550, Some(s)), 375);
        assert_eq!(beside(&[MAIN], 550, None), 375);
        // Near the right edge: the left side.
        let edge = Rect { x: 1100, ..MAIN };
        assert_eq!(beside(&[edge], 300, Some(s)), 800);
        // Fits on neither side: the right, as before.
        assert_eq!(beside(&[Rect { x: 200, w: 1100, ..MAIN }], 400, Some(s)), 1300);
    }

    #[test]
    fn snaps_beside_and_to_the_screen() {
        // 6 px right of main's right edge: pulled in.
        let m = Rect { x: 381, y: 120, w: 275, h: 116 };
        assert_eq!(snap(&[m], &[MAIN], None, 10), (-6, 0));
        // Within reach on both axes: tops line up too.
        let near = Rect { x: 381, y: 104, w: 275, h: 116 };
        assert_eq!(snap(&[near], &[MAIN], None, 10), (-6, -4));
        // Far away: nothing.
        let far = Rect { x: 800, y: 600, w: 275, h: 116 };
        assert_eq!(snap(&[far], &[MAIN], None, 10), (0, 0));
        // Near the screen's left edge.
        let s = Rect { x: 0, y: 0, w: 1440, h: 900 };
        let edge = Rect { x: 7, y: 400, w: 275, h: 116 };
        assert_eq!(snap(&[edge], &[], Some(s), 10), (-7, 0));
    }
}
