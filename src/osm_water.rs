//! Water from OpenStreetMap: lakes and rivers drawn as areas (`natural=water`,
//! `waterway=riverbank`, also as multipolygons with their islands). The ground inside each
//! is lowered below its banks, sloping down from them, and the tiles it lies on get OMSI's
//! water a little under the banks' height, so it shows where the ground was lowered.

use glam::DVec2;

use crate::doc::{tile_origin, tile_size, Document, TileKey};
use crate::history::History;
use crate::osm_green::{inside, rings};

/// Water areas and their islands (latitude, longitude; closed rings).
#[derive(Clone, Debug, Default)]
pub struct Water {
    pub areas: Vec<Vec<(f64, f64)>>,
    pub islands: Vec<Vec<(f64, f64)>>,
}

pub fn query(bbox: [f64; 4]) -> String {
    let b = format!("{:.6},{:.6},{:.6},{:.6}", bbox[0], bbox[1], bbox[2], bbox[3]);
    format!("[out:json][timeout:90];(way[\"natural\"=\"water\"]({b});way[\"waterway\"=\"riverbank\"]({b});rel[\"natural\"=\"water\"]({b});rel[\"waterway\"=\"riverbank\"]({b}););out geom;")
}

pub fn parse(body: &str) -> Result<Water, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    parse_value(&v, &|| Ok(()))
}

pub(crate) fn parse_value(v: &serde_json::Value, check: &impl Fn() -> Result<(), String>) -> Result<Water, String> {
    check()?;
    let mut w = Water::default();
    let pts = |g: Option<&serde_json::Value>| -> Vec<(f64, f64)> { g.and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect() };
    for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
        check()?;
        if e["tags"]["natural"] != "water" && e["tags"]["waterway"] != "riverbank" { continue; }
        match e.get("type").and_then(|t| t.as_str()) {
            Some("way") => {
                let p = pts(e.get("geometry"));
                if p.len() >= 4 && p.first() == p.last() {
                    w.areas.push(p);
                }
            }
            Some("relation") => {
                let (mut outer, mut inner) = (Vec::new(), Vec::new());
                for m in e.get("members").and_then(|m| m.as_array()).into_iter().flatten() {
                    check()?;
                    let p = pts(m.get("geometry"));
                    if p.len() < 2 {
                        continue;
                    }
                    match m.get("role").and_then(|r| r.as_str()) {
                        Some("inner") => inner.push(p),
                        _ => outer.push(p),
                    }
                }
                w.areas.extend(rings(outer));
                w.islands.extend(rings(inner));
            }
            _ => {}
        }
    }
    Ok(w)
}

/// How far `p` is from the ring's edge.
fn edge_distance(poly: &[DVec2], p: DVec2) -> f64 {
    poly.windows(2)
        .map(|w| {
            let d = w[1] - w[0];
            let t = ((p - w[0]).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
            (p - (w[0] + d * t)).length()
        })
        .fold(f64::MAX, f64::min)
}

/// The ground lowered `depth` m below the banks inside the areas (reaching it `slope` m in
/// from the edge), and water on the tiles they lie on 0.3 m under the banks; one undo step.
/// How many tiles got water.
pub fn apply(doc: &mut Document, hist: &mut History, areas: &[Vec<DVec2>], islands: &[Vec<DVec2>], depth: f64) -> usize {
    const SLOPE: f64 = 6.0;
    hist.begin(crate::i18n::tr("Water from OpenStreetMap", "Вода из OpenStreetMap"));
    let mut wet: Vec<(TileKey, f32)> = Vec::new();
    for area in areas {
        // the banks' height: the ground along the edge within the map, every 5 m
        let mut banks = Vec::new();
        for w in area.windows(2) {
            let l = (w[1] - w[0]).length();
            let n = (l / 5.0).ceil().max(1.0) as usize;
            for i in 0..n {
                let p = w[0] + (w[1] - w[0]) * (i as f64 / n as f64);
                if let Some(h) = doc.height_at(p) {
                    banks.push(h as f64);
                }
            }
        }
        if banks.is_empty() {
            continue;
        }
        banks.sort_by(|a, b| a.total_cmp(b));
        // (the lower banks: the water's edge, not the top of a slope beside it)
        let bank = banks[banks.len() / 4];
        let bottom = bank - depth;
        let (lo, hi) = area.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().filter(|k| {
            let o = tile_origin(*k);
            o.x <= hi.x && o.y <= hi.y && o.x + tile_size() >= lo.x && o.y + tile_size() >= lo.y
        }).collect();
        for k in keys {
            let org = tile_origin(k);
            let mut touched = false;
            let t = doc.tiles.get(&k).unwrap();
            let s = t.terrain.samples();
            let cell = tile_size() / (s - 1).max(1) as f64;
            let mut lowered: Vec<(usize, f32)> = Vec::new();
            for j in 0..s {
                for i in 0..s {
                    let p = org + DVec2::new(i as f64, j as f64) * cell;
                    if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y || !inside(area, p) || islands.iter().any(|isl| inside(isl, p)) {
                        continue;
                    }
                    let into = edge_distance(area, p);
                    let want = bank + (bottom - bank) * (into / SLOPE).min(1.0);
                    let h = t.terrain.heights[j * s + i] as f64;
                    if want < h {
                        lowered.push((j * s + i, want as f32));
                    }
                    touched = true;
                }
            }
            if !touched {
                continue;
            }
            hist.touch_tile(doc, k);
            let t = doc.tiles.get_mut(&k).unwrap();
            for (at, h) in lowered {
                t.terrain.heights[at] = h;
            }
            t.touch_terrain();
            let level = (bank - 0.3) as f32;
            match wet.iter_mut().find(|w| w.0 == k) {
                Some(w) => w.1 = w.1.min(level),
                None => wet.push((k, level)),
            }
        }
    }
    for (k, level) in &wet {
        if let Some(t) = doc.tiles.get_mut(k) {
            let now = t.water.map(|w| w.iter().copied().fold(f32::MAX, f32::min));
            t.water = Some([now.map(|n| n.min(*level)).unwrap_or(*level); 4]);
            t.touch_content();
        }
    }
    if wet.is_empty() {
        hist.cancel();
    } else {
        hist.commit();
    }
    wet.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pond_is_dug_and_filled() {
        let _grid = crate::doc::one_map();
        let mut doc = crate::selection::tests::flat_map(2);
        for t in doc.tiles.values_mut() {
            t.terrain.heights.iter_mut().for_each(|h| *h = 10.0);
        }
        let pond = vec![DVec2::new(100.0, 100.0), DVec2::new(200.0, 100.0), DVec2::new(200.0, 200.0), DVec2::new(100.0, 200.0), DVec2::new(100.0, 100.0)];
        let mut hist = History::default();
        assert_eq!(apply(&mut doc, &mut hist, &[pond], &[], 2.0), 1);
        let t = &doc.tiles[&(0, 0)];
        assert!((t.height_at(150.0, 150.0) - 8.0).abs() < 1e-3, "{}", t.height_at(150.0, 150.0));
        assert!((t.height_at(50.0, 50.0) - 10.0).abs() < 1e-3);
        assert_eq!(t.water, Some([9.7; 4]));
        assert!(doc.tiles[&(1, 0)].water.is_none());
        hist.undo(&mut doc);
        assert!(doc.tiles[&(0, 0)].water.is_none());
    }
}
