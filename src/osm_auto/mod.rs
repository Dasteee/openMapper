//! Automatic, inspectable OSM asset planning. Workers only read geography and installed
//! content; file generation and map mutation happen after an explicit Build action.

pub mod buildings;
pub mod index;
mod occupancy;
pub mod roads;
#[cfg(test)]
mod tests;
mod write;

use crate::{
    doc::{Document, tile_of, tile_origin},
    install::Roots,
    library::Asset,
    osm::{Class, Road},
};
use glam::DVec2;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

#[derive(Clone, Debug)]
pub enum RoadOption {
    Installed(index::RoadAsset),
    Generated(roads::Target),
}
impl RoadOption {
    pub fn label(&self) -> String {
        match self {
            Self::Installed(a) => a.rel.clone(),
            Self::Generated(t) => format!(
                "{} · {}+{} · {:.1} m",
                crate::i18n::tr("Procedural fallback", "Процедурное дополнение"),
                t.forward,
                t.backward,
                t.width
            ),
        }
    }
    pub fn key(&self) -> String {
        match self {
            Self::Installed(a) => format!("installed/{}", a.rel.to_lowercase()),
            Self::Generated(_) => "procedural".into(),
        }
    }
    pub fn half_width(&self) -> f64 {
        match self {
            Self::Installed(a) => a.bounds[0].abs().max(a.bounds[1].abs()),
            Self::Generated(t) => (t.width + t.left_walk + t.right_walk) * 0.5,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoadChoice {
    pub road: Road,
    pub candidates: Vec<RoadOption>,
    pub selected: usize,
    pub reason: String,
}
impl RoadChoice {
    pub fn key(&self) -> String {
        format!("{:?}/{:?}", self.road.source.ids, self.road.points.first())
    }
}

#[derive(Clone, Debug)]
pub struct ExistingObject {
    pub rel: String,
    pub at: DVec2,
    pub heading: f64,
}

pub struct Spec {
    pub key: String,
    pub revision: u64,
    pub roots: Roots,
    pub assets: Vec<Asset>,
    pub roads: Vec<Road>,
    pub buildings: Vec<buildings::Footprint>,
    pub overrides: HashMap<Class, String>,
    pub segment_overrides: HashMap<String, String>,
    pub building_dirs: Vec<String>,
    pub procedural: bool,
    pub existing_roads: Vec<(omsi_geometry::SplineCurve, f64)>,
    pub existing_objects: Vec<ExistingObject>,
    pub tiles: Vec<crate::doc::TileKey>,
}

pub struct Plan {
    pub key: String,
    pub revision: u64,
    pub roads: Vec<RoadChoice>,
    pub buildings: Vec<buildings::Placement>,
    pub warnings: Vec<String>,
    pub catalogue: (usize, usize, usize, usize),
    pub base_occupied: Vec<Vec<DVec2>>,
    pub dependencies: Vec<index::FileStamp>,
    pub skipped_roads: usize,
    pub skipped_buildings: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Catalogue,
    Objects,
    Roads,
    Buildings,
}
impl Phase {
    pub fn label(self) -> &'static str {
        use crate::i18n::tr;
        match self {
            Self::Catalogue => tr("Asset catalogue", "Каталог ресурсов"),
            Self::Objects => tr("Objects", "Объекты"),
            Self::Roads => tr("Roads", "Дороги"),
            Self::Buildings => tr("Buildings", "Здания"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: Phase,
    pub processed: usize,
    pub total: usize,
    pub detail: String,
    pub elapsed: std::time::Duration,
}
impl Progress {
    pub fn remaining_seconds(&self) -> Option<u64> {
        (self.processed >= 5 && self.processed < self.total && self.elapsed.as_secs_f64() >= 1.0)
            .then(|| (self.elapsed.as_secs_f64() / self.processed as f64 * (self.total - self.processed) as f64).ceil() as u64)
    }
}
pub enum Msg {
    Progress(Progress),
    Done(Result<Plan, String>),
}
pub struct Job {
    pub rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(spec: Spec) -> Result<Job, String> {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::Builder::new()
        .name("osm-plan".into())
        .spawn(move || {
            let check = || if flag.load(Ordering::Relaxed) { Err("Cancelled".into()) } else { Ok(()) };
            let stage = std::cell::Cell::new((Phase::Catalogue, std::time::Instant::now()));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                plan(spec, &check, &|phase, i, n, name| {
                    if stage.get().0 != phase {
                        stage.set((phase, std::time::Instant::now()));
                    }
                    let _ = tx.send(Msg::Progress(Progress {
                        phase,
                        processed: i,
                        total: n,
                        detail: name.into(),
                        elapsed: stage.get().1.elapsed(),
                    }));
                })
            }))
            .unwrap_or_else(|_| Err("OSM planner worker panicked".into()));
            match &result {
                Ok(plan) => log::info!("OSM preview ready: {} roads, {} buildings, {} unresolved roads, {} unresolved buildings", plan.roads.len(), plan.buildings.len(), plan.skipped_roads, plan.skipped_buildings),
                Err(error) => log::error!("OSM automatic preview failed: {error}"),
            }
            let _ = tx.send(Msg::Done(result));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { rx, cancel })
}

fn road_corridor(a: DVec2, b: DVec2, width: f64) -> Vec<DVec2> {
    let d = (b - a).normalize_or_zero();
    let side = DVec2::new(-d.y, d.x) * width;
    vec![a + side, b + side, b - side, a - side]
}

fn road_occupancy(road: &Road, half: f64) -> Vec<Vec<DVec2>> {
    let mut out = Vec::new();
    for segment in crate::osm::segments(&road.points, road.class) {
        let curve = segment.curve();
        let n = (segment.length / 3.0).ceil().max(1.0) as usize;
        for i in 0..n {
            out.push(road_corridor(
                curve.point_at(segment.length * i as f64 / n as f64).truncate(),
                curve.point_at(segment.length * (i + 1) as f64 / n as f64).truncate(),
                half + 0.5,
            ));
        }
    }
    out
}

fn junction_occupancy(choices: &[RoadChoice]) -> Vec<Vec<DVec2>> {
    let mut nodes: HashMap<i64, Vec<(DVec2, DVec2, f64)>> = HashMap::new();
    for choice in choices {
        if choice.road.class.is_track() {
            continue;
        }
        let half = choice.candidates[choice.selected].half_width();
        for (i, node) in choice.road.source.nodes.iter().enumerate() {
            let Some(p) = choice.road.points.get(i).copied() else {
                continue;
            };
            for j in [i.checked_sub(1), i.checked_add(1).filter(|j| *j < choice.road.points.len())]
                .into_iter()
                .flatten()
            {
                let direction = (choice.road.points[j] - p).normalize_or_zero();
                if direction.length_squared() > 0.5 {
                    nodes.entry(*node).or_default().push((p, direction, half));
                }
            }
        }
    }
    nodes
        .into_values()
        .filter_map(|mut arms| {
            arms.sort_by(|a, b| a.1.x.total_cmp(&b.1.x).then_with(|| a.1.y.total_cmp(&b.1.y)));
            arms.dedup_by(|a, b| (a.1 - b.1).length() < 0.001);
            if arms.len() < 3 {
                return None;
            }
            let angle = arms
                .iter()
                .enumerate()
                .flat_map(|(i, a)| arms.iter().skip(i + 1).map(move |b| a.1.dot(b.1).clamp(-1.0, 1.0).acos().to_degrees()))
                .fold(180.0, f64::min);
            let widths: Vec<_> = arms.iter().map(|a| a.2).collect();
            let radius = crate::junction::radius_at(&widths, angle) + 0.5;
            let at = arms[0].0;
            Some(
                (0..32)
                    .map(|i| at + DVec2::from_angle(i as f64 * std::f64::consts::TAU / 32.0) * radius)
                    .collect(),
            )
        })
        .collect()
}

pub fn existing(doc: &Document, lib: &crate::library::Library) -> (Vec<(omsi_geometry::SplineCurve, f64)>, Vec<ExistingObject>) {
    let mut roads = Vec::new();
    let mut objects = Vec::new();
    for (k, tile) in &doc.tiles {
        for s in tile.tile.splines.iter().filter(|s| !s.deleted) {
            let width = lib.find(&s.file).map(|a| a.half_width as f64).unwrap_or(4.0) + 0.5;
            roads.push((omsi_geometry::SplineCurve::from_map(s, tile_origin(*k)), width));
        }
        for o in &tile.tile.objects {
            objects.push(ExistingObject {
                rel: o.file.clone(),
                at: tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1]),
                heading: o.rot[0],
            });
        }
    }
    (roads, objects)
}

fn plan(spec: Spec, check: &impl Fn() -> Result<(), String>, progress: &impl Fn(Phase, usize, usize, &str)) -> Result<Plan, String> {
    let assets: Vec<_> = spec
        .assets
        .iter()
        .filter(|a| !spec.buildings.is_empty() || a.kind != crate::library::Kind::Object)
        .cloned()
        .collect();
    let index = index::build(&assets, &spec.roots, &spec.building_dirs, check, &|i, n, name| {
        progress(Phase::Catalogue, i, n, name)
    })?;
    plan_with_index(spec, index, check, progress)
}

fn plan_with_index(
    spec: Spec,
    index: index::Index,
    check: &impl Fn() -> Result<(), String>,
    progress: &impl Fn(Phase, usize, usize, &str),
) -> Result<Plan, String> {
    let mut result = Plan {
        revision: spec.revision,
        key: spec.key,
        roads: Vec::new(),
        buildings: Vec::new(),
        warnings: Vec::new(),
        catalogue: (index.roads.len(), index.buildings.len(), index.reused, index.rejected),
        skipped_roads: 0,
        skipped_buildings: 0,
        base_occupied: vec![],
        dependencies: vec![],
    };
    let mut occupied = occupancy::Occupancy::default();
    let objects_total = spec.existing_roads.len() + if spec.buildings.is_empty() { 0 } else { spec.existing_objects.len() };
    for (i, (curve, width)) in spec.existing_roads.iter().enumerate() {
        check()?;
        progress(Phase::Objects, i, objects_total, "");
        let n = (curve.length / 4.0).ceil().max(1.0) as usize;
        for i in 0..n {
            occupied.insert(road_corridor(
                curve.point_at(curve.length * i as f64 / n as f64).truncate(),
                curve.point_at(curve.length * (i + 1) as f64 / n as f64).truncate(),
                *width,
            ));
        }
    }
    let mut existing_sizes: HashMap<String, Option<index::BuildingAsset>> = HashMap::new();
    for (i, o) in spec.existing_objects.iter().filter(|_| !spec.buildings.is_empty()).enumerate() {
        check()?;
        progress(Phase::Objects, spec.existing_roads.len() + i, objects_total, &o.rel);
        let known = index.buildings.iter().find(|a| a.rel.eq_ignore_ascii_case(&o.rel)).cloned();
        let asset = existing_sizes.entry(o.rel.to_lowercase()).or_insert_with(|| {
            known.or_else(|| {
                let model = crate::assets::load_object(&spec.roots.resolve(&o.rel), &spec.roots.omsi, &spec.roots.content)?;
                if model.technical || model.tree.is_some() {
                    return None;
                }
                Some(index::BuildingAsset {
                    rel: o.rel.clone(),
                    lo: model.lo.as_dvec3().to_array(),
                    hi: model.hi.as_dvec3().to_array(),
                    usage: String::new(),
                })
            })
        });
        if let Some(a) = asset {
            occupied.insert(buildings::rectangle(a, o.at, o.heading));
        }
    }
    result.base_occupied = occupied.polygons().to_vec();
    progress(Phase::Objects, objects_total, objects_total, "");
    let roads_total = spec.roads.len();
    for (i, road) in spec.roads.into_iter().enumerate() {
        check()?;
        progress(Phase::Roads, i, roads_total, &format!("{:?}", road.source.ids));
        let target = match roads::target(&road) {
            Ok(t) => t,
            Err(e) => {
                result.warnings.push(format!("Road {:?}: {e}", road.source.ids));
                result.skipped_roads += 1;
                continue;
            }
        };
        let ranked = roads::rank(&target, &index.roads, road.class.is_track());
        let mut candidates: Vec<_> = ranked.iter().map(|(_, a)| RoadOption::Installed(a.clone())).collect();
        if spec.procedural && !road.class.is_track() {
            candidates.push(RoadOption::Generated(target.clone()));
        }
        if let Some(rel) = road.asset.as_ref().or_else(|| spec.overrides.get(&road.class)) {
            if let Some(asset) = index.roads.iter().find(|a| a.rel.eq_ignore_ascii_case(rel)) {
                if roads::rank(&target, &[asset.clone()], road.class.is_track()).is_empty() {
                    result.warnings.push(format!(
                        "Road {:?}: override {} conflicts with its lane direction, width or mode",
                        road.source.ids, rel
                    ));
                    result.skipped_roads += 1;
                    continue;
                }
                candidates.retain(|a| !matches!(a,RoadOption::Installed(a)if a.rel.eq_ignore_ascii_case(rel)));
                candidates.insert(0, RoadOption::Installed(asset.clone()));
            } else {
                result.warnings.push(format!(
                    "Road {:?}: override {} is unreadable or has missing dependencies",
                    road.source.ids, rel
                ));
                result.skipped_roads += 1;
                continue;
            }
        }
        if candidates.is_empty() {
            result.warnings.push(format!("Road {:?}: no compatible installed spline", road.source.ids));
            result.skipped_roads += 1;
            continue;
        }
        // Strong dimension/sidewalk mismatch prefers the procedural cross-section.
        let mut selected = if road.asset.is_some() || spec.overrides.contains_key(&road.class) {
            0
        } else if spec.procedural && !road.class.is_track() && ranked.first().is_none_or(|a| a.0 > 0.7) {
            candidates.len() - 1
        } else {
            0
        };
        let key = format!("{:?}/{:?}", road.source.ids, road.points.first());
        if let Some(label) = spec.segment_overrides.get(&key) {
            if let Some(i) = candidates.iter().position(|a| a.key() == *label) {
                selected = i;
            } else {
                result
                    .warnings
                    .push(format!("Road {:?}: previous override is no longer compatible", road.source.ids));
                result.skipped_roads += 1;
                continue;
            }
        }
        occupied.extend(road_occupancy(&road, candidates[selected].half_width()));
        let reason = format!(
            "{}+{} lanes{}; {:.1} m carriageway; {} dimensions",
            target.forward,
            target.backward,
            if target.shared { " shared" } else { "" },
            target.width,
            if target.inferred { "estimated" } else { "OSM" }
        );
        result.roads.push(RoadChoice {
            road,
            candidates,
            selected,
            reason,
        });
    }
    progress(Phase::Roads, roads_total, roads_total, "");
    occupied.extend(junction_occupancy(&result.roads));
    let mut source = spec.buildings;
    source.sort_by(|a, b| a.id.cmp(&b.id));
    let total = source.len();
    let catalogue = buildings::Catalogue::new(&index.buildings);
    let tile_bounds: Vec<_> = spec
        .tiles
        .iter()
        .map(|key| {
            let lo = tile_origin(*key);
            occupancy::Bounds {
                lo,
                hi: lo + DVec2::splat(crate::doc::tile_size()),
            }
        })
        .collect();
    for (processed, f) in source.into_iter().enumerate() {
        check()?;
        progress(Phase::Buildings, processed, total, &f.id);
        if occupancy::Bounds::of(&f.outer).is_some_and(|bounds| !tile_bounds.iter().any(|tile| bounds.intersects(*tile))) {
            result.warnings.push(format!("Building {}: outside selected tiles", f.id));
            result.skipped_buildings += 1;
            continue;
        }
        let id = f.id.clone();
        match buildings::choose_indexed(&f, &catalogue, &occupied, spec.procedural, check) {
            Ok(p) => {
                if !spec.tiles.contains(&tile_of(p.at)) || p.footprint.iter().any(|p| !spec.tiles.contains(&tile_of(*p))) {
                    result.warnings.push(format!("Building {id}: footprint crosses unselected tiles"));
                    result.skipped_buildings += 1;
                    continue;
                }
                occupied.insert(p.footprint.clone());
                result.buildings.push(p);
            }
            Err(e) => {
                result.warnings.push(format!("Building {id}: {e}"));
                result.skipped_buildings += 1;
            }
        }
    }
    progress(Phase::Buildings, total, total, "");
    let mut dependencies = std::collections::BTreeMap::new();
    let road_assets = result.roads.iter().flat_map(|r| {
        r.candidates.iter().filter_map(|a| match a {
            RoadOption::Installed(a) => Some(a.rel.as_str()),
            _ => None,
        })
    });
    for rel in road_assets.chain(result.buildings.iter().filter_map(|b| b.asset.as_deref())) {
        if let Some(files) = index.files.get(rel) {
            for file in files {
                dependencies.insert(file.path.clone(), file.clone());
            }
        }
    }
    result.dependencies = dependencies.into_values().collect();
    Ok(result)
}

/// Include document revisions in the plan key: a preview cannot be applied after edits.
pub fn revision(doc: &Document) -> u64 {
    let tiles: Vec<_> = doc.tiles.iter().map(|(key, tile)| (key, tile.content_rev, tile.terrain_rev)).collect();
    crate::geo::fetch::fnv(&serde_json::to_vec(&(doc.dir.to_string_lossy(), doc.global_rev, tiles)).unwrap_or_default())
}

pub use write::prepare;

impl crate::editor::Editor {
    pub fn place_osm_planned_buildings(&mut self, placements: &[buildings::Placement]) -> usize {
        let Some(doc) = self.doc.as_mut() else {
            return 0;
        };
        self.history.begin(crate::i18n::tr("Buildings from OpenStreetMap", "Здания из OpenStreetMap"));
        self.history.touch_global(doc);
        let mut count = 0;
        for p in placements {
            let Some(file) = &p.asset else {
                continue;
            };
            let key = tile_of(p.at);
            if !doc.tiles.contains_key(&key) {
                continue;
            }
            self.history.touch_tile(doc, key);
            let id = doc.new_id();
            let origin = tile_origin(key);
            let object = omsi_map::MapObject {
                file: file.clone(),
                id,
                pos: [p.at.x - origin.x, p.at.y - origin.y, p.base],
                rot: [p.heading, 0.0, 0.0],
                ..Default::default()
            };
            let tile = doc.tiles.get_mut(&key).unwrap();
            tile.tile.objects.push(object);
            tile.touch_content();
            count += 1;
        }
        if count == 0 {
            self.history.cancel();
        } else {
            self.history.commit();
        }
        count
    }
}
