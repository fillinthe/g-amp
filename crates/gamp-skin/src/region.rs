//! REGION.TXT: window shapes as polygons. Pixels outside are transparent.

use std::collections::HashMap;

/// Section name (lower case: "normal", "windowshade", "equalizer",
/// "equalizerws") → polygons.
pub type Regions = HashMap<String, Vec<Vec<(i32, i32)>>>;

pub fn parse(text: &str) -> Regions {
    let mut out = Regions::new();
    let mut section = String::new();
    let mut counts: Vec<usize> = Vec::new();
    let mut points: Vec<i32> = Vec::new();
    let flush = |section: &str, counts: &mut Vec<usize>, points: &mut Vec<i32>, out: &mut Regions| {
        if section.is_empty() || counts.is_empty() {
            counts.clear();
            points.clear();
            return;
        }
        let mut polys = Vec::new();
        let mut i = 0;
        for c in counts.iter() {
            let mut poly = Vec::new();
            for _ in 0..*c {
                if i + 1 < points.len() {
                    poly.push((points[i], points[i + 1]));
                }
                i += 2;
            }
            if poly.len() >= 3 {
                polys.push(poly);
            }
        }
        out.insert(section.to_owned(), polys);
        counts.clear();
        points.clear();
    };
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        if line.starts_with('[') {
            flush(&section, &mut counts, &mut points, &mut out);
            section = line.trim_matches(['[', ']']).trim().to_ascii_lowercase();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let nums = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).filter_map(|p| p.parse::<i32>().ok());
        match k.trim().to_ascii_lowercase().as_str() {
            "numpoints" => counts.extend(nums.map(|n| n.max(0) as usize)),
            "pointlist" => points.extend(nums),
            _ => {}
        }
    }
    flush(&section, &mut counts, &mut points, &mut out);
    out
}

/// A mask for a `width`×`height` window: true where the window is.
pub fn mask(polys: &[Vec<(i32, i32)>], width: u32, height: u32) -> Vec<bool> {
    let mut m = vec![false; (width * height) as usize];
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            // Pixel centres, even-odd rule over all polygons.
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut inside = false;
            for poly in polys {
                let n = poly.len();
                let mut j = n - 1;
                for i in 0..n {
                    let (xi, yi) = (poly[i].0 as f32, poly[i].1 as f32);
                    let (xj, yj) = (poly[j].0 as f32, poly[j].1 as f32);
                    if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
            }
            m[(y as u32 * width + x as u32) as usize] = inside;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    #[test]
    fn rectangle_with_a_notch() {
        let r = super::parse("[Normal]\nNumPoints=4\nPointList=0,0, 275,0, 275,116, 0,116\n[WindowShade]\nNumPoints=4\nPointList=0,0,275,0,275,14,0,14");
        assert_eq!(r["normal"][0].len(), 4);
        let m = super::mask(&r["normal"], 275, 116);
        assert!(m[0] && m[275 * 116 - 1]);
        let tri = vec![vec![(0, 0), (10, 0), (0, 10)]];
        let m = super::mask(&tri, 10, 10);
        assert!(m[0] && !m[99]);
    }
}
