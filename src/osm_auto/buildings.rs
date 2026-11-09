use super::index::BuildingAsset;
use super::occupancy::{Bounds, Occupancy};
use glam::{DVec2, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Footprint {
    pub id: String,
    pub tags: std::collections::BTreeMap<String, String>,
    pub outer: Vec<DVec2>,
    pub holes: Vec<Vec<DVec2>>,
    pub height: f64,
    pub min_height: f64,
    pub known: bool,
    pub usage: String,
    pub issue: Option<String>,
}

pub fn area(p: &[DVec2]) -> f64 {
    edges(p).map(|(a, b)| a.perp_dot(b)).sum::<f64>() * 0.5
}
pub fn edges(p: &[DVec2]) -> impl Iterator<Item = (DVec2, DVec2)> + '_ {
    p.iter().copied().enumerate().map(|(i, a)| (a, p[(i + 1) % p.len()]))
}

pub fn distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    (p - (a + d * ((p - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0))).length()
}

pub fn contains(p: DVec2, ring: &[DVec2]) -> bool {
    let mut inside = false;
    for (a, b) in edges(ring) {
        if distance(p, a, b) < 1e-6 {
            return true;
        }
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

fn crosses(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let side = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    side(a, b, c) * side(a, b, d) < -1e-8 && side(c, d, a) * side(c, d, b) < -1e-8
}

pub fn overlaps(a: &[DVec2], b: &[DVec2]) -> bool {
    if !Bounds::of(a).zip(Bounds::of(b)).is_some_and(|(a, b)| a.intersects(b)) {
        return false;
    }
    let strictly_inside = |p: DVec2, ring: &[DVec2]| contains(p, ring) && edges(ring).all(|(a, b)| distance(p, a, b) > 0.01);
    edges(a).any(|(a, b0)| edges(b).any(|(c, d)| crosses(a, b0, c, d)))
        || a.iter().any(|p| strictly_inside(*p, b))
        || b.iter().any(|p| strictly_inside(*p, a))
        || (a.len() > 2 && strictly_inside(a.iter().copied().sum::<DVec2>() / a.len() as f64, b))
}

fn clean(p: &[DVec2]) -> Vec<DVec2> {
    let mut out = Vec::new();
    for p in p {
        if out.last().is_none_or(|q: &DVec2| (*q - *p).length() > 1e-5) {
            out.push(*p);
        }
    }
    if out.len() > 1 && (out[0] - out[out.len() - 1]).length() < 1e-5 {
        out.pop();
    }
    out
}

pub fn validate(f: &Footprint) -> Result<(), String> {
    if let Some(issue) = &f.issue {
        return Err(issue.clone());
    }
    if !f.height.is_finite() || !f.min_height.is_finite() || f.height <= f.min_height || f.height > 300.0 {
        return Err("Invalid or unsupported building height".into());
    }
    if f.outer.len() + f.holes.iter().map(Vec::len).sum::<usize>() > 8192 {
        return Err("Footprint exceeds the procedural geometry budget".into());
    }
    let outer = clean(&f.outer);
    for ring in std::iter::once(&outer).chain(f.holes.iter()) {
        let ring = clean(ring);
        if ring.len() < 3 || ring.len() > 2048 || ring.iter().any(|p| !p.is_finite()) || area(&ring).abs() < 1.0 {
            return Err("Invalid or oversized footprint".into());
        }
        let segments: Vec<_> = edges(&ring).collect();
        for (i, (a, b)) in segments.iter().enumerate() {
            for (j, (c, d)) in segments.iter().enumerate().skip(i + 1) {
                if j != i + 1 && !(i == 0 && j == segments.len() - 1) && crosses(*a, *b, *c, *d) {
                    return Err("Self-intersecting footprint".into());
                }
            }
        }
    }
    for (i, hole) in f.holes.iter().enumerate() {
        if hole.iter().any(|p| !contains(*p, &outer)) || edges(hole).any(|(a, b)| edges(&outer).any(|(c, d)| crosses(a, b, c, d))) {
            return Err("Courtyard lies outside its parent".into());
        }
        if f.holes[..i].iter().any(|h| overlaps(h, hole)) {
            return Err("Overlapping courtyards".into());
        }
    }
    Ok(())
}

pub fn rectangle(asset: &BuildingAsset, at: DVec2, heading: f64) -> Vec<DVec2> {
    let x = crate::roads::right(heading);
    let y = crate::roads::dir(heading);
    [
        (asset.lo[0], asset.lo[1]),
        (asset.hi[0], asset.lo[1]),
        (asset.hi[0], asset.hi[1]),
        (asset.lo[0], asset.hi[1]),
    ]
    .iter()
    .map(|(a, b)| at + x * *a + y * *b)
    .collect()
}

fn fits(rect: &[DVec2], f: &Footprint) -> bool {
    if rect.iter().any(|p| !contains(*p, &f.outer)) {
        return false;
    }
    // Split a candidate edge at every source-boundary intersection. Testing each open
    // interval catches narrow concave notches without sampling every half metre.
    for (a, b) in edges(rect) {
        let d = b - a;
        let mut cuts = vec![0.0, 1.0];
        for (c, e) in edges(&f.outer) {
            let edge = e - c;
            let denominator = d.perp_dot(edge);
            if denominator.abs() > 1e-10 {
                let t = (c - a).perp_dot(edge) / denominator;
                let u = (c - a).perp_dot(d) / denominator;
                if (-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u) {
                    cuts.push(t.clamp(0.0, 1.0));
                }
            } else if (c - a).perp_dot(d).abs() < 1e-8 {
                let length = d.length_squared().max(1e-12);
                cuts.push(((c - a).dot(d) / length).clamp(0.0, 1.0));
                cuts.push(((e - a).dot(d) / length).clamp(0.0, 1.0));
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        if cuts.windows(2).any(|cut| !contains(a.lerp(b, (cut[0] + cut[1]) * 0.5), &f.outer)) {
            return false;
        }
    }
    !f.holes.iter().any(|h| overlaps(rect, h) || h.iter().any(|p| contains(*p, rect)))
}

pub struct Catalogue<'a> {
    assets: &'a [BuildingAsset],
    by_area: Vec<(f64, usize)>,
}
impl<'a> Catalogue<'a> {
    pub fn new(assets: &'a [BuildingAsset]) -> Self {
        let mut by_area: Vec<_> = assets
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let area = (a.hi[0] - a.lo[0]) * (a.hi[1] - a.lo[1]);
                (area.is_finite() && area > 0.0).then_some((area, i))
            })
            .collect();
        by_area.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Self { assets, by_area }
    }
    fn candidates(&self, area: f64) -> impl Iterator<Item = &'a BuildingAsset> + '_ {
        let lo = self.by_area.partition_point(|(a, _)| *a < area * 0.7 - 1e-8);
        let hi = self.by_area.partition_point(|(a, _)| *a <= area * 1.01 + 1e-8);
        self.by_area[lo..hi].iter().map(|(_, i)| &self.assets[*i])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    pub source: Footprint,
    pub asset: Option<String>,
    pub at: DVec2,
    pub heading: f64,
    pub base: f64,
    pub footprint: Vec<DVec2>,
    pub reason: String,
}

#[cfg(test)]
pub fn choose(f: &Footprint, assets: &[BuildingAsset], occupied: &[Vec<DVec2>], procedural: bool) -> Result<Placement, String> {
    choose_indexed(
        f,
        &Catalogue::new(assets),
        &Occupancy::from_polygons(occupied.iter().cloned()),
        procedural,
        &|| Ok(()),
    )
}

pub fn choose_indexed(
    f: &Footprint,
    catalogue: &Catalogue<'_>,
    occupied: &Occupancy,
    procedural: bool,
    check: &impl Fn() -> Result<(), String>,
) -> Result<Placement, String> {
    check()?;
    validate(f)?;
    let heading = crate::osm_buildings::own_heading(&f.outer);
    let mut choices = Vec::new();
    let nearby = occupied.nearby(&f.outer);
    let want_area = area(&f.outer).abs() - f.holes.iter().map(|h| area(h).abs()).sum::<f64>();
    let dimensions = crate::osm_buildings::footprint(&f.outer, heading);
    for asset in catalogue.candidates(want_area) {
        check()?;
        if !f.usage.is_empty() && !asset.usage.is_empty() && asset.usage != f.usage {
            continue;
        }
        let h = asset.hi[2] - asset.lo[2];
        if f.known && (h / f.height - 1.0).abs() > 0.3 {
            continue;
        }
        for rotate in [0.0, 90.0, 180.0, 270.0] {
            let (width, depth) = if rotate == 90.0 || rotate == 270.0 {
                (dimensions.2, dimensions.1)
            } else {
                (dimensions.1, dimensions.2)
            };
            if asset.hi[0] - asset.lo[0] > width + 1e-6 || asset.hi[1] - asset.lo[1] > depth + 1e-6 {
                continue;
            }
            let heading = crate::roads::norm_deg(heading + rotate);
            let center = dimensions.0;
            let at = center - crate::roads::right(heading) * (asset.lo[0] + asset.hi[0]) * 0.5 - crate::roads::dir(heading) * (asset.lo[1] + asset.hi[1]) * 0.5;
            let rect = rectangle(asset, at, heading);
            let coverage = area(&rect).abs() / want_area.max(1.0);
            if !(0.7..=1.01).contains(&coverage) || !fits(&rect, f) || nearby.iter().any(|p| overlaps(p, &rect)) {
                continue;
            }
            let score = (1.0 - coverage).abs() + (h / f.height).ln().abs() * if f.known { 0.8 } else { 0.3 } + if asset.usage.is_empty() { 0.15 } else { 0.0 };
            choices.push((score, asset, at, heading, rect));
        }
    }
    choices.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.rel.cmp(&b.1.rel)).then_with(|| a.3.total_cmp(&b.3)));
    if let Some(best) = choices.first() {
        let count = choices.iter().take_while(|c| c.0 <= best.0 + 0.025).count();
        let pick = &choices[crate::geo::fetch::fnv(f.id.as_bytes()) as usize % count];
        return Ok(Placement {
            source: f.clone(),
            asset: Some(pick.1.rel.clone()),
            at: pick.2,
            heading: pick.3,
            base: f.min_height - pick.1.lo[2],
            footprint: pick.4.clone(),
            reason: format!(
                "Installed model; footprint coverage {:.0}%; {} height",
                area(&pick.4).abs() / want_area * 100.0,
                if f.known { "OSM tags" } else { "estimated" }
            ),
        });
    }
    if !procedural {
        return Err("No installed building fits the footprint and clearances".into());
    }
    check()?;
    if nearby
        .iter()
        .any(|p| overlaps(p, &f.outer) && !f.holes.iter().any(|hole| p.iter().all(|point| contains(*point, hole))))
    {
        return Err("Footprint intersects a road or another building".into());
    }
    let at = f.outer.iter().copied().sum::<DVec2>() / f.outer.len().max(1) as f64;
    Ok(Placement {
        source: f.clone(),
        asset: None,
        at,
        heading: 0.0,
        base: f.min_height,
        footprint: f.outer.clone(),
        reason: "Procedural footprint extrusion; no suitable installed model".into(),
    })
}

pub fn mesh(f: &Footprint, origin: DVec2) -> Result<openmapper_mesh::Mesh, String> {
    validate(f)?;
    let mut rings: Vec<_> = std::iter::once(&f.outer).chain(f.holes.iter()).map(|r| clean(r)).collect();
    for (i, r) in rings.iter_mut().enumerate() {
        if (area(r) > 0.0) != (i == 0) {
            r.reverse();
        }
    }
    let mut flat = Vec::new();
    let mut holes = Vec::new();
    let mut points = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        if i > 0 {
            holes.push(points.len());
        }
        for p in ring {
            let p = *p - origin;
            flat.extend([p.x, p.y]);
            points.push(p);
        }
    }
    let tris = earcutr::earcut(&flat, &holes, 2).map_err(|e| format!("Roof triangulation: {e:?}"))?;
    let expected = area(&rings[0]).abs() - rings.iter().skip(1).map(|h| area(h).abs()).sum::<f64>();
    let actual = tris
        .chunks_exact(3)
        .map(|t| (points[t[1]] - points[t[0]]).perp_dot(points[t[2]] - points[t[0]]).abs() * 0.5)
        .sum::<f64>();
    if (actual - expected).abs() > expected * 1e-5 {
        return Err("Roof triangulation does not preserve footprint area".into());
    }
    let height = (f.height - f.min_height) as f32;
    let mut mesh = openmapper_mesh::Mesh::default();
    for p in &points {
        mesh.positions.push(Vec3::new(p.x as f32, p.y as f32, 0.0));
    }
    for p in &points {
        mesh.positions.push(Vec3::new(p.x as f32, p.y as f32, height));
    }
    let n = points.len() as u32;
    let uv = |p: DVec2| Vec2::new((p.x / 8.0) as f32, (p.y / 8.0) as f32);
    for tri in tris.chunks_exact(3) {
        let mut t = [tri[0], tri[1], tri[2]];
        if (points[t[1]] - points[t[0]]).perp_dot(points[t[2]] - points[t[0]]) < 0.0 {
            t.swap(1, 2);
        }
        mesh.faces.push(openmapper_mesh::Face::new(
            t.iter().map(|i| *i as u32 + n).collect(),
            t.iter().map(|i| uv(points[*i])).collect(),
            1,
        ));
        mesh.faces.push(openmapper_mesh::Face::new(
            t.iter().rev().map(|i| *i as u32).collect(),
            t.iter().rev().map(|i| uv(points[*i])).collect(),
            1,
        ));
    }
    let mut offset = 0u32;
    for ring in &rings {
        for i in 0..ring.len() {
            let a = offset + i as u32;
            let b = offset + ((i + 1) % ring.len()) as u32;
            let width = (ring[(i + 1) % ring.len()] - ring[i]).length() as f32 / 4.0;
            mesh.faces.push(openmapper_mesh::Face::new(
                vec![a, b, b + n, a + n],
                vec![Vec2::ZERO, Vec2::new(width, 0.0), Vec2::new(width, height / 3.0), Vec2::new(0.0, height / 3.0)],
                0,
            ));
        }
        offset += ring.len() as u32;
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<DVec2> {
        vec![DVec2::new(x, y), DVec2::new(x + w, y), DVec2::new(x + w, y + h), DVec2::new(x, y + h)]
    }
    fn footprint() -> Footprint {
        Footprint {
            id: "way/42".into(),
            tags: Default::default(),
            outer: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![],
            height: 9.0,
            min_height: 0.0,
            known: true,
            usage: "house".into(),
            issue: None,
        }
    }
    #[test]
    fn exact_edge_checks_reject_a_notch_narrower_than_the_old_sampling_step() {
        let mut f = footprint();
        f.outer = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(20.0, 0.0),
            DVec2::new(20.0, 20.0),
            DVec2::new(9.79, 20.0),
            DVec2::new(9.79, 5.0),
            DVec2::new(9.71, 5.0),
            DVec2::new(9.71, 20.0),
            DVec2::new(0.0, 20.0),
        ];
        let rect = rect(0.0, 0.0, 20.0, 10.0);
        assert!(!fits(&rect, &f));
        assert!(fits(&rect, &footprint()));
    }

    #[test]
    fn area_shortlist_and_inner_loop_cancellation_are_bounded() {
        let assets: Vec<_> = (1..=1000)
            .map(|n| BuildingAsset {
                rel: format!("{n}.sco"),
                lo: [0.0; 3],
                hi: [n as f64, n as f64, 9.0],
                usage: "house".into(),
            })
            .collect();
        let catalogue = Catalogue::new(&assets);
        assert_eq!(catalogue.candidates(400.0).count(), 4);
        let candidates = vec![assets[19].clone(); 100];
        let catalogue = Catalogue::new(&candidates);
        let calls = std::cell::Cell::new(0);
        let result = choose_indexed(&footprint(), &catalogue, &Occupancy::default(), true, &|| {
            calls.set(calls.get() + 1);
            if calls.get() >= 4 { Err("cancelled during matching".into()) } else { Ok(()) }
        });
        assert!(result.unwrap_err().contains("cancelled during matching"));
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn courtyard_remains_empty_in_the_generated_roof() {
        let mut f = footprint();
        f.holes.push(rect(5.0, 5.0, 10.0, 10.0));
        let mesh = mesh(&f, DVec2::ZERO).unwrap();
        let roof: Vec<_> = mesh
            .faces
            .iter()
            .filter(|face| face.verts.iter().all(|i| mesh.positions[*i as usize].z > 8.0))
            .collect();
        let sum: f64 = roof
            .iter()
            .map(|face| {
                let p: Vec<_> = face.verts.iter().map(|i| mesh.positions[*i as usize].truncate().as_dvec2()).collect();
                area(&p).abs()
            })
            .sum();
        assert!((sum - 300.0).abs() < 1e-6);
        for face in roof {
            let c = face.verts.iter().map(|i| mesh.positions[*i as usize].truncate().as_dvec2()).sum::<DVec2>() / 3.0;
            assert!(!contains(c, &f.holes[0]));
        }
    }
    #[test]
    fn an_offset_model_fits_without_stretching_and_collisions_are_rejected() {
        let f = footprint();
        let a = BuildingAsset {
            rel: "house.sco".into(),
            lo: [10.0, 5.0, -2.0],
            hi: [30.0, 25.0, 7.0],
            usage: "house".into(),
        };
        let p = choose(&f, &[a.clone()], &[], true).unwrap();
        assert_eq!(p.asset.as_deref(), Some("house.sco"));
        assert_eq!(p.base, 2.0);
        assert!(choose(&f, &[a], &[rect(4.0, 4.0, 2.0, 2.0)], true).is_err());
        let mut bad = f;
        bad.outer = vec![DVec2::ZERO, DVec2::new(10.0, 10.0), DVec2::new(0.0, 10.0), DVec2::new(10.0, 0.0)];
        assert!(validate(&bad).is_err());
    }
}
