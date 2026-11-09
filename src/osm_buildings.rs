//! Buildings from OpenStreetMap: each building's outline filled with the model of a chosen
//! catalogue folder whose ground plan comes nearest to it, standing with its front to the
//! nearest road (or along the outline's longest side where no road is near).

use glam::DVec2;
use std::collections::{BTreeMap, HashSet};

use crate::roads;

pub fn query(bbox: [f64; 4]) -> String {
    let b = format!("{:.6},{:.6},{:.6},{:.6}", bbox[0], bbox[1], bbox[2], bbox[3]);
    format!("[out:json][timeout:90];(way[\"building\"]({b});way[\"building:part\"]({b});rel[\"building\"][\"type\"=\"multipolygon\"]({b}););out geom;")
}

/// A building of OpenStreetMap: its outline (latitude, longitude; closed) and how high it
/// is (m) and whether OSM says so (else guessed from what kind of building it is).
#[derive(Clone, Debug)]
pub struct Building {
    pub id: String,
    pub tags: BTreeMap<String, String>,
    pub outline: Vec<(f64, f64)>,
    pub holes: Vec<Vec<(f64, f64)>>,
    pub min_height: f64,
    pub issue: Option<String>,
    pub height: f64,
    pub known: bool,
}

/// How high a building is: its `height`, or its storeys, or what its kind usually is.
fn height_of(tag: &dyn Fn(&str) -> String) -> (f64, bool) {
    let num = |k: &str| measure(&tag(k)).filter(|v| *v > 0.0);
    if let Some(h) = num("height") {
        return (h, true);
    }
    if let Some(l) = num("building:levels") {
        return (l * 3.0 + num("roof:levels").unwrap_or(0.0) * 2.5 + 1.5, true);
    }
    let guess = match tag("building").as_str() {
        "house" | "detached" | "semidetached_house" | "bungalow" | "farm" | "terrace" => 8.0,
        "garage" | "garages" | "shed" | "hut" | "carport" | "kiosk" | "roof" => 3.5,
        "apartments" | "residential" => 15.0,
        "industrial" | "warehouse" | "retail" | "supermarket" => 8.0,
        _ => 11.0,
    };
    (guess, false)
}

pub fn parse(body: &str) -> Result<Vec<Building>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    parse_value(&v, &|| Ok(()))
}

pub(crate) fn parse_value(v: &serde_json::Value, check: &impl Fn() -> Result<(), String>) -> Result<Vec<Building>, String> {
    check()?;
    let elements = v.get("elements").and_then(|v| v.as_array()).map(Vec::as_slice).unwrap_or_default();
    let points = |v: &serde_json::Value| -> Vec<(f64, f64)> {
        v.get("geometry").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect()
    };
    let make = |e: &serde_json::Value, id: String, outline, holes, issue| {
        let tags: BTreeMap<String, String> = e.get("tags").and_then(|v| v.as_object()).into_iter().flatten().filter_map(|(k,v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
        let tag = |key: &str| tags.get(key).cloned().unwrap_or_default();
        let (height, known) = height_of(&tag);
        let min_height = measure(&tag("min_height")).or_else(|| measure(&tag("building:min_level")).map(|n| n * 3.0)).unwrap_or(0.0);
        Building { id, tags, outline, holes, min_height, height, known, issue }
    };
    let mut out = Vec::new();
    let mut members = HashSet::new();
    for relation in elements.iter().filter(|e| e["type"] == "relation" && (!e["tags"]["building"].is_null() || !e["tags"]["building:part"].is_null())) {
        check()?;
        let mut outer = Vec::new(); let mut inner = Vec::new();
        for member in relation.get("members").and_then(|v| v.as_array()).into_iter().flatten() {
            if member["type"] != "way" { continue; }
            if let Some(id) = member["ref"].as_i64() { members.insert(id); }
            if member["role"] == "inner" { inner.push(points(member)); } else { outer.push(points(member)); }
        }
        let (outer, complete_outer) = rings(outer);
        let (inner, complete_inner) = rings(inner);
        let had_outer = !outer.is_empty();
        let complete = complete_outer && complete_inner && had_outer && inner.iter().all(|h|h.first().is_some_and(|p|outer.iter().any(|o|inside_geo(*p,o))));
        for (i, ring) in outer.into_iter().enumerate() {
            let holes = inner.iter().filter(|hole| hole.first().is_some_and(|p| inside_geo(*p, &ring))).cloned().collect();
            out.push(make(relation, format!("relation/{}/{}", relation["id"], i), ring, holes, (!complete).then(|| "Incomplete multipolygon".into())));
        }
        if !had_outer {
            out.push(make(relation, format!("relation/{}", relation["id"]), vec![], vec![], Some("Incomplete multipolygon".into())));
        }
    }
    for way in elements.iter().filter(|e| e["type"] == "way") {
        check()?;
        if members.contains(&way["id"].as_i64().unwrap_or(0)) || (way["tags"]["building"].is_null() && way["tags"]["building:part"].is_null()) { continue; }
        let ring = points(way);
        let issue = (ring.len() < 4 || ring.first() != ring.last()).then(|| "Open or incomplete building outline".into());
        out.push(make(way, format!("way/{}", way["id"]), ring, vec![], issue));
    }
    // A parent footprint is an envelope when actual building parts cover it.
    let parts: Vec<_> = out.iter().filter(|b| b.tags.get("building:part").is_some_and(|s| s != "no")).map(|b| b.outline.clone()).collect();
    out.retain(|b| b.tags.contains_key("building:part") || !parts.iter().any(|p| p.first().is_some_and(|p| inside_geo(*p, &b.outline))));
    Ok(out)
}

pub fn measure(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', ".");
    let (s, scale) = if let Some(s) = text.strip_suffix("ft").or_else(|| text.strip_suffix('\'')) { (s, 0.3048) } else { (text.trim_end_matches('m'), 1.0) };
    s.trim().parse::<f64>().ok().map(|n| n * scale).filter(|n| n.is_finite() && *n >= 0.0)
}

fn inside_geo(p: (f64, f64), ring: &[(f64,f64)]) -> bool {
    let mut inside = false;
    for edge in ring.windows(2) { let (a,b) = (edge[0],edge[1]); if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0-a.0)*(p.1-a.1)/(b.1-a.1)+a.0 { inside = !inside; } }
    inside
}

fn rings(mut fragments: Vec<Vec<(f64,f64)>>) -> (Vec<Vec<(f64,f64)>>, bool) {
    let mut result = Vec::new(); let mut complete = true;
    fragments.retain(|p| p.len() >= 2);
    while let Some(mut ring) = fragments.pop() {
        while ring.first() != ring.last() {
            let next = fragments.iter().position(|p| p.first() == ring.last() || p.last() == ring.last());
            let Some(i) = next else { complete = false; break; };
            let mut part = fragments.remove(i);
            if part.last() == ring.last() { part.reverse(); }
            ring.extend(part.into_iter().skip(1));
        }
        if ring.len() >= 4 && ring.first() == ring.last() { result.push(ring); }
    }
    (result, complete)
}

/// An outline seen standing with heading `h` (front towards `dir(h)`): its middle, its
/// width across (x) and its depth (y).
pub fn footprint(poly: &[DVec2], h: f64) -> (DVec2, f64, f64) {
    let (x, y) = (roads::right(h), roads::dir(h));
    let c = poly.iter().copied().sum::<DVec2>() / poly.len().max(1) as f64;
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in poly {
        let d = *p - c;
        let (a, b) = (d.dot(x), d.dot(y));
        x0 = x0.min(a);
        x1 = x1.max(a);
        y0 = y0.min(b);
        y1 = y1.max(b);
    }
    (c + x * (x0 + x1) * 0.5 + y * (y0 + y1) * 0.5, x1 - x0, y1 - y0)
}

/// The heading an outline stands with where no road is near: along its longest side.
pub fn own_heading(poly: &[DVec2]) -> f64 {
    let longest = poly.windows(2).max_by(|a, b| (a[1] - a[0]).length().total_cmp(&(b[1] - b[0]).length()));
    // (the front across the long side: the long side is the width)
    longest.map(|w| roads::norm_deg(roads::heading_of(w[1] - w[0]) + 90.0)).unwrap_or(0.0)
}

/// How far a model of `size` (width, depth, height) is from a building (0: the same); None
/// when its ground plan is too far off to stand there. The height counts less, and less
/// still when it is only guessed (`known` false).
pub fn misfit(size: (f64, f64, f64), want: (f64, f64, f64), known: bool) -> Option<f64> {
    let (rx, ry) = (size.0 / want.0.max(0.1), size.1 / want.1.max(0.1));
    let ok = |r: f64| (0.6..=1.7).contains(&r);
    let rz = (size.2 / want.2.max(1.0)).ln().abs();
    (ok(rx) && ok(ry)).then(|| rx.ln().abs() + ry.ln().abs() + rz * if known { 0.8 } else { 0.4 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipolygons_keep_courtyards_and_do_not_duplicate_member_ways() {
        let body=r#"{"elements":[{"type":"relation","id":9,"tags":{"type":"multipolygon","building":"apartments","height":"30 ft","min_height":"3"},"members":[{"type":"way","ref":1,"role":"outer","geometry":[{"lat":0,"lon":0},{"lat":0,"lon":20},{"lat":20,"lon":20}]},{"type":"way","ref":2,"role":"outer","geometry":[{"lat":0,"lon":0},{"lat":20,"lon":0},{"lat":20,"lon":20}]},{"type":"way","ref":3,"role":"inner","geometry":[{"lat":5,"lon":5},{"lat":5,"lon":10},{"lat":10,"lon":10},{"lat":10,"lon":5},{"lat":5,"lon":5}]}]},{"type":"way","id":3,"tags":{"building":"yes"},"geometry":[{"lat":5,"lon":5},{"lat":5,"lon":10},{"lat":10,"lon":10},{"lat":10,"lon":5},{"lat":5,"lon":5}]}]}"#;
        let buildings=parse(body).unwrap();assert_eq!(buildings.len(),1);assert_eq!(buildings[0].holes.len(),1);assert!(buildings[0].issue.is_none());assert!((buildings[0].height-9.144).abs()<1e-6);assert_eq!(buildings[0].min_height,3.0);
    }

    #[test]
    fn an_outline_is_measured_across_its_front() {
        // 20 m along x, 10 m along y, its middle at (110, 205)
        let sq = vec![DVec2::new(100.0, 200.0), DVec2::new(120.0, 200.0), DVec2::new(120.0, 210.0), DVec2::new(100.0, 210.0), DVec2::new(100.0, 200.0)];
        // facing north (+y): 20 across, 10 deep
        let (c, w, d) = footprint(&sq, 0.0);
        assert!((c - DVec2::new(110.0, 205.0)).length() < 1e-9 && (w - 20.0).abs() < 1e-9 && (d - 10.0).abs() < 1e-9);
        // facing east: the other way round
        let (_, w, d) = footprint(&sq, 90.0);
        assert!((w - 10.0).abs() < 1e-9 && (d - 20.0).abs() < 1e-9);
        assert!((own_heading(&sq) % 180.0 - 90.0).abs() < 1e-9 || (own_heading(&sq) % 180.0).abs() < 1e-9);
        assert!(misfit((19.0, 11.0, 9.0), (20.0, 10.0, 9.0), true).unwrap() < 0.2);
        assert!(misfit((40.0, 10.0, 9.0), (20.0, 10.0, 9.0), true).is_none());
        // a tower where a house stands: further than a house of the same plan
        assert!(misfit((20.0, 10.0, 60.0), (20.0, 10.0, 8.0), false).unwrap() > misfit((20.0, 10.0, 9.0), (20.0, 10.0, 8.0), false).unwrap() + 0.5);
        let body = r#"{"elements":[{"type":"way","tags":{"building":"yes","building:levels":"4"},"geometry":[{"lat":0,"lon":0},{"lat":0,"lon":1},{"lat":1,"lon":1},{"lat":0,"lon":0}]}]}"#;
        let b = parse(body).unwrap();
        assert!(b[0].known && (b[0].height - 13.5).abs() < 1e-9);
    }
}
