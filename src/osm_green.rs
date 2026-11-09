//! Trees and woods from OpenStreetMap: single trees (`natural=tree`), rows of them
//! (`natural=tree_row`) and woods (`landuse=forest`, `natural=wood`), planted with the
//! plant mix where they are, as the plant brush plants: a random one of the mix, turned a
//! random way, none closer than the spacing, none on a road.

use glam::DVec2;

/// What OpenStreetMap has (latitude, longitude).
#[derive(Clone, Debug, Default)]
pub struct Green {
    pub trees: Vec<(f64, f64)>,
    pub rows: Vec<Vec<(f64, f64)>>,
    pub woods: Vec<Vec<(f64, f64)>>,
    /// Clearings in them (a multipolygon's inner rings).
    pub holes: Vec<Vec<(f64, f64)>>,
}

pub fn query(bbox: [f64; 4]) -> String {
    let b = format!("{:.6},{:.6},{:.6},{:.6}", bbox[0], bbox[1], bbox[2], bbox[3]);
    format!("[out:json][timeout:90];(node[\"natural\"=\"tree\"]({b});way[\"natural\"=\"tree_row\"]({b});way[\"landuse\"=\"forest\"]({b});way[\"natural\"=\"wood\"]({b});rel[\"landuse\"=\"forest\"]({b});rel[\"natural\"=\"wood\"]({b}););out geom;")
}

pub fn parse(body: &str) -> Result<Green, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    parse_value(&v, &|| Ok(()))
}

pub(crate) fn parse_value(v: &serde_json::Value, check: &impl Fn() -> Result<(), String>) -> Result<Green, String> {
    check()?;
    let mut g = Green::default();
    for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
        check()?;
        let tag = |k: &str| e.get("tags").and_then(|t| t.get(k)).and_then(|v| v.as_str()).unwrap_or("");
        match e.get("type").and_then(|t| t.as_str()) {
            Some("node") if tag("natural") == "tree" => {
                if let (Some(la), Some(lo)) = (e.get("lat").and_then(|x| x.as_f64()), e.get("lon").and_then(|x| x.as_f64())) {
                    g.trees.push((la, lo));
                }
            }
            Some("way") => {
                let pts: Vec<(f64, f64)> = e.get("geometry").and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect();
                if pts.len() < 2 {
                    continue;
                }
                if tag("natural") == "tree_row" {
                    g.rows.push(pts);
                } else if (tag("landuse") == "forest" || tag("natural") == "wood") && pts.len() >= 4 && pts.first() == pts.last() {
                    g.woods.push(pts);
                }
            }
            Some("relation") if tag("landuse") == "forest" || tag("natural") == "wood" => {
                // a multipolygon: its outer and inner ways joined into rings
                let mut outer = Vec::new();
                let mut inner = Vec::new();
                for m in e.get("members").and_then(|m| m.as_array()).into_iter().flatten() {
                    check()?;
                    let pts: Vec<(f64, f64)> = m.get("geometry").and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect();
                    if pts.len() < 2 {
                        continue;
                    }
                    match m.get("role").and_then(|r| r.as_str()) {
                        Some("inner") => inner.push(pts),
                        _ => outer.push(pts),
                    }
                }
                g.woods.extend(rings(outer));
                g.holes.extend(rings(inner));
            }
            _ => {}
        }
    }
    Ok(g)
}

/// Ways joined end to end into closed rings (what does not close is left out).
pub(crate) fn rings(mut ways: Vec<Vec<(f64, f64)>>) -> Vec<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    while let Some(mut ring) = ways.pop() {
        while ring.first() != ring.last() {
            let end = *ring.last().unwrap();
            let Some(i) = ways.iter().position(|w| w.first() == Some(&end) || w.last() == Some(&end)) else { break };
            let mut w = ways.swap_remove(i);
            if w.first() != Some(&end) {
                w.reverse();
            }
            ring.extend(w.into_iter().skip(1));
        }
        if ring.len() >= 4 && ring.first() == ring.last() {
            out.push(ring);
        }
    }
    out
}

/// Numbers 0..1 for shaking the trees about (xorshift: the same wood comes out the same).
pub fn shaker() -> impl FnMut() -> f64 {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Inside a polygon (even-odd).
pub fn inside(poly: &[DVec2], p: DVec2) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
    }
    c
}

/// The polygon's area (m²).
#[cfg(test)]
pub fn area(poly: &[DVec2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i].perp_dot(poly[(i + 1) % n])).sum::<f64>().abs() * 0.5
}

/// Where trees go: each tree; along each row every `spacing`; in each wood (out of its
/// clearings) one in each `spacing` square, shaken about within it (`rng` 0..1) so they
/// stand in no grid; only within `clip` (the map's box: a forest is far bigger).
pub fn spots(trees: &[DVec2], rows: &[Vec<DVec2>], woods: &[Vec<DVec2>], holes: &[Vec<DVec2>], clip: (DVec2, DVec2), spacing: f64, rng: &mut impl FnMut() -> f64) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = trees.to_vec();
    for r in rows {
        let mut left = 0.0;
        for w in r.windows(2) {
            let d = w[1] - w[0];
            let l = d.length();
            let mut s = left;
            while s < l {
                out.push(w[0] + d / l.max(1e-9) * s);
                s += spacing;
            }
            left = s - l;
        }
    }
    for w in woods {
        let (lo, hi) = w.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
        let (lo, hi) = (lo.max(clip.0), hi.min(clip.1));
        let mut y = lo.y;
        while y < hi.y {
            let mut x = lo.x;
            while x < hi.x {
                let p = DVec2::new(x + rng() * spacing, y + rng() * spacing);
                if inside(w, p) && !holes.iter().any(|h| inside(h, p)) {
                    out.push(p);
                }
                x += spacing;
            }
            y += spacing;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wood_is_filled_and_a_row_planted_along() {
        let sq = vec![DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0), DVec2::new(100.0, 100.0), DVec2::new(0.0, 100.0), DVec2::new(0.0, 0.0)];
        assert!((area(&sq) - 10_000.0).abs() < 1e-6);
        assert!(inside(&sq, DVec2::new(50.0, 50.0)) && !inside(&sq, DVec2::new(150.0, 50.0)));
        let mut k = 0.0;
        let mut rng = || {
            k = (k + 0.37) % 1.0;
            k
        };
        let all = (DVec2::splat(-1e9), DVec2::splat(1e9));
        let s = spots(&[], &[vec![DVec2::new(0.0, 200.0), DVec2::new(95.0, 200.0)]], &[sq.clone()], &[], all, 10.0, &mut rng);
        let row = s.iter().filter(|p| p.y == 200.0).count();
        assert_eq!(row, 10);
        let wood = s.len() - row;
        assert!((90..=100).contains(&wood), "{wood}");
        let body = r#"{"elements":[{"type":"node","lat":1.0,"lon":2.0,"tags":{"natural":"tree"}},{"type":"way","tags":{"landuse":"forest"},"geometry":[{"lat":0,"lon":0},{"lat":0,"lon":1},{"lat":1,"lon":1},{"lat":0,"lon":0}]}]}"#;
        let g = parse(body).unwrap();
        assert_eq!((g.trees.len(), g.woods.len()), (1, 1));
        // a clearing, and the map's box
        let hole = vec![DVec2::new(0.0, 0.0), DVec2::new(50.0, 0.0), DVec2::new(50.0, 100.0), DVec2::new(0.0, 100.0)];
        let s = spots(&[], &[], &[sq.clone()], &[hole], all, 10.0, &mut rng);
        assert!(s.iter().all(|p| p.x >= 50.0) && s.len() > 40);
        let s = spots(&[], &[], &[sq], &[], (DVec2::ZERO, DVec2::splat(30.0)), 10.0, &mut rng);
        assert!(s.len() <= 9 && s.iter().all(|p| p.x < 40.0));
        // a multipolygon of two halves
        let body = r#"{"elements":[{"type":"relation","tags":{"landuse":"forest"},"members":[{"type":"way","role":"outer","geometry":[{"lat":0,"lon":0},{"lat":0,"lon":1},{"lat":1,"lon":1}]},{"type":"way","role":"outer","geometry":[{"lat":0,"lon":0},{"lat":1,"lon":1}]}]}]}"#;
        assert_eq!(parse(body).unwrap().woods.len(), 1);
    }
}
