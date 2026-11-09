//! Roads from OpenStreetMap: the streets of the map's place fetched with the Overpass API
//! (the map must lie on the Earth: Satellite and terrain), the ways of one street joined
//! into one road, ordered so a road is built before the ones that end on it (they meet it
//! in its middle: a T junction), and each polyline made of straights and arcs as OMSI's
//! splines are (each corner rounded as wide as its legs allow).

use std::collections::{BTreeMap, HashMap};
use serde::{Deserialize, Serialize};

use glam::DVec2;

use crate::roads::{self, Seg};

/// What a road is for, from its `highway` tag (or a track, from its `railway` tag): each
/// class gets a road or track type of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// Motorways, trunk, primary and secondary roads (and their links).
    Main,
    /// Tertiary, residential, unclassified and living streets.
    Street,
    /// Service roads (yards, car parks, depots).
    Service,
    /// Tram and light rail tracks.
    Tram,
    /// Railway tracks.
    Rail,
}

impl Class {
    pub const ALL: [Class; 5] = [Class::Main, Class::Street, Class::Service, Class::Tram, Class::Rail];

    /// Tracks: no junctions with what they meet (crossings stay level); a branch gets a
    /// switch with Auto-Switch on.
    pub fn is_track(self) -> bool {
        matches!(self, Class::Tram | Class::Rail)
    }

    pub fn of_railway(railway: &str) -> Option<Class> {
        match railway {
            "tram" | "light_rail" => Some(Class::Tram),
            "rail" => Some(Class::Rail),
            _ => None,
        }
    }

    pub fn of(highway: &str) -> Option<Class> {
        let h = highway.strip_suffix("_link").unwrap_or(highway);
        match h {
            "motorway" | "trunk" | "primary" | "secondary" => Some(Class::Main),
            "tertiary" | "residential" | "unclassified" | "living_street" => Some(Class::Street),
            "service" => Some(Class::Service),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        use crate::i18n::tr;
        match self {
            Class::Main => tr("Main roads", "Главные дороги"),
            Class::Street => tr("Streets", "Улицы"),
            Class::Service => tr("Service roads", "Служебные проезды"),
            Class::Tram => tr("Tram tracks", "Трамвайные пути"),
            Class::Rail => tr("Railway tracks", "Железнодорожные пути"),
        }
    }

    pub fn hint(self) -> &'static str {
        use crate::i18n::tr;
        match self {
            Class::Main => tr("motorway, trunk, primary, secondary", "магистрали, главные и второстепенные"),
            Class::Street => tr("tertiary, residential, unclassified", "местные и жилые улицы"),
            Class::Service => tr("yards, car parks, depots", "дворы, стоянки, депо"),
            Class::Tram => tr("each track on its own; branches are joined with switches by hand", "каждый путь отдельно; ответвления соединяются стрелками вручную"),
            Class::Rail => tr("main lines without sidings; branches are joined with switches by hand", "магистрали без подъездных; ответвления соединяются стрелками вручную"),
        }
    }

    /// The radius a corner of such a road is rounded with when its legs leave room (m).
    fn corner_radius(self) -> f64 {
        match self {
            Class::Main => 60.0,
            Class::Street => 20.0,
            Class::Service => 10.0,
            Class::Tram => 25.0,
            Class::Rail => 300.0,
        }
    }
}

/// One way of OpenStreetMap: its nodes' ids and where they are (latitude, longitude).
#[derive(Clone, Debug)]
pub struct Way {
    pub id: i64,
    pub tags: BTreeMap<String, String>,
    pub reversed: bool,
    pub restrictions: Vec<Restriction>,
    pub class: Class,
    /// Traffic one way only (`oneway`, a roundabout, a motorway): one carriageway of a
    /// dual road, a one-way street.
    pub oneway: bool,
    pub nodes: Vec<i64>,
    pub geometry: Vec<(f64, f64)>,
}

/// The Overpass query for the roads and bus stops within a box (south, west, north, east).
pub fn query(bbox: [f64; 4]) -> String {
    let b = format!("{:.6},{:.6},{:.6},{:.6}", bbox[0], bbox[1], bbox[2], bbox[3]);
    format!("[out:json][timeout:90];(way[\"railway\"~\"^(tram|light_rail|rail)$\"][\"service\"!~\"^(yard|siding|spur)$\"]({b});way[\"highway\"~\"^(motorway|trunk|primary|secondary|tertiary|unclassified|residential|living_street|service)(_link)?$\"][\"area\"!=\"yes\"]({b});node[\"highway\"=\"bus_stop\"]({b});rel[\"type\"=\"restriction\"]({b}););out geom;")
}

/// A bus stop of OpenStreetMap: where (latitude, longitude) and its name.
#[derive(Clone, Debug)]
pub struct Stop {
    pub at: (f64, f64),
    pub name: String,
}

/// The bus stops of an Overpass answer.
pub fn parse_stops(body: &str) -> Result<Vec<Stop>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    parse_stops_value(&v, &|| Ok(()))
}

pub(crate) fn parse_stops_value(v: &serde_json::Value, check: &impl Fn() -> Result<(), String>) -> Result<Vec<Stop>, String> {
    check()?;
    Ok(v.get("elements")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("node") && (e["tags"]["highway"] == "bus_stop" || (e["tags"]["public_transport"] == "platform" && e["tags"]["bus"] == "yes")))
        .filter_map(|e| {
            let name = e.get("tags").and_then(|t| t.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string();
            Some(Stop { at: (e.get("lat")?.as_f64()?, e.get("lon")?.as_f64()?), name })
        })
        .collect())
}

/// The ways of an Overpass answer (`out geom`).
pub fn parse(body: &str) -> Result<Vec<Way>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    parse_value(&v, &|| Ok(()))
}

pub(crate) fn parse_value(v: &serde_json::Value, check: &impl Fn() -> Result<(), String>) -> Result<Vec<Way>, String> {
    check()?;
    let restrictions: Vec<Restriction> = v.get("elements").and_then(|e|e.as_array()).into_iter().flatten().filter(|e|e["type"]=="relation"&&e["tags"]["type"]=="restriction").filter_map(|e|{
        let members=e.get("members")?.as_array()?;
        let member=|role:&str|members.iter().find(|m|m["role"]==role).and_then(|m|m["ref"].as_i64());
        Some(Restriction{id:e["id"].as_i64()?,from:member("from")?,to:member("to")?,via:member("via"),kind:e["tags"]["restriction"].as_str().unwrap_or("unsupported").into()})
    }).collect();
    let mut out = Vec::new();
    for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
        check()?;
        if e.get("type").and_then(|t| t.as_str()) != Some("way") {
            continue;
        }
        let tag = |k: &str| e.get("tags").and_then(|t| t.get(k)).and_then(|v| v.as_str()).unwrap_or("");
        let Some(class) = Class::of(tag("highway")).or_else(|| Class::of_railway(tag("railway"))) else { continue };
        let oneway = match tag("oneway") { "no" | "0" | "false" => false, "yes" | "1" | "true" | "-1" => true, _ => tag("junction") == "roundabout" || tag("highway") == "motorway" };
        let mut nodes: Vec<i64> = e.get("nodes").and_then(|n| n.as_array()).into_iter().flatten().filter_map(|n| n.as_i64()).collect();
        let mut geometry: Vec<(f64, f64)> = e.get("geometry").and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect();
        if nodes.len() < 2 || nodes.len() != geometry.len() {
            continue;
        }
        let reversed = tag("oneway") == "-1";
        if reversed { nodes.reverse(); geometry.reverse(); }
        let tags = e.get("tags").and_then(|v| v.as_object()).into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect();
        out.push(Way { id: e.get("id").and_then(|v| v.as_i64()).unwrap_or(0), tags, reversed, restrictions: restrictions.iter().filter(|r|r.from==e["id"].as_i64().unwrap_or(0)||r.to==e["id"].as_i64().unwrap_or(0)).cloned().collect(), class, oneway, nodes, geometry });
    }
    Ok(out)
}

/// A road to build: its class and its points (world), in the way OSM runs.
#[derive(Clone, Debug)]
pub struct Road {
    pub class: Class,
    pub oneway: bool,
    pub points: Vec<DVec2>,
    pub source: RoadSource,
    /// Per-segment choice from the automatic planner; manual class mappings remain valid.
    pub asset: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoadSource {
    pub restrictions: Vec<Restriction>,
    pub ids: Vec<i64>,
    pub nodes: Vec<i64>,
    pub tags: BTreeMap<String, String>,
    pub reversed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Restriction { pub id:i64, pub from:i64, pub to:i64, pub via:Option<i64>, pub kind:String }

impl RoadSource {
    pub fn tag(&self, key: &str) -> &str { self.tags.get(key).map(String::as_str).unwrap_or("") }
    pub fn directional(&self, key: &str, forward: bool) -> &str {
        let side = if forward != self.reversed { "forward" } else { "backward" };
        self.tags.get(&format!("{key}:{side}")).map(String::as_str).unwrap_or("")
    }
}

/// The ways put into world coordinates (`to_world`), the parts outside the map (`inside`)
/// cut off, the ways of one street joined end to end, and ordered: a road others end on is
/// built before them, main roads before streets.
pub fn roads(ways: &[Way], to_world: impl Fn(f64, f64) -> DVec2, inside: impl Fn(DVec2) -> bool) -> Vec<Road> {
    // node → world point
    let mut at: HashMap<i64, DVec2> = HashMap::new();
    for w in ways {
        for (n, g) in w.nodes.iter().zip(&w.geometry) {
            at.entry(*n).or_insert_with(|| to_world(g.0, g.1));
        }
    }
    // the parts of each way within the map: its runs of nodes inside, from and to where it
    // crosses the map's edge (new nodes there, numbered below zero)
    let mut parts: Vec<(Kind, Vec<i64>, Vec<i64>)> = Vec::new();
    let mut fresh = -1i64;
    let mut edge = |at: &mut HashMap<i64, DVec2>, a: DVec2, b: DVec2| -> i64 {
        // (a inside, b outside: halved down to a centimetre, then a little inside)
        let (mut lo, mut hi) = (a, b);
        for _ in 0..30 {
            let m = (lo + hi) * 0.5;
            if inside(m) { lo = m } else { hi = m }
        }
        let back = (a - lo).normalize_or_zero() * 0.5;
        let id = fresh;
        fresh -= 1;
        at.insert(id, lo + back);
        id
    };
    for w in ways {
        let signature = serde_json::to_vec(&(&w.tags, w.reversed)).unwrap_or_default();
        let kind = Kind { class: w.class, oneway: w.oneway, signature: crate::geo::fetch::fnv(&signature), directional: !w.tags.is_empty() };
        let mut run: Vec<i64> = Vec::new();
        for (i, n) in w.nodes.iter().enumerate() {
            let p = at[n];
            let prev = i.checked_sub(1).map(|j| at[&w.nodes[j]]);
            if inside(p) {
                if let Some(q) = prev.filter(|q| !inside(*q)) {
                    run.push(edge(&mut at, p, q));
                }
                run.push(*n);
            } else {
                if let Some(q) = prev.filter(|q| inside(*q)) {
                    run.push(edge(&mut at, q, p));
                }
                if run.len() >= 2 {
                    parts.push((kind, std::mem::take(&mut run), vec![w.id]));
                }
                run.clear();
            }
        }
        if run.len() >= 2 {
            parts.push((kind, run, vec![w.id]));
        }
    }
    let parts = join(parts, &at);
    let mut roads = order(parts, &at);
    for road in &mut roads {
        let sources: Vec<_> = ways.iter().filter(|w| road.source.ids.contains(&w.id)).collect();
        if let Some(first) = sources.first() {
            road.source.tags = first.tags.clone();
            road.source.reversed = first.reversed;
            road.source.restrictions=sources.iter().flat_map(|w|w.restrictions.clone()).collect();
            road.source.ids = sources.iter().map(|w| w.id).collect();
            road.source.ids.sort_unstable(); road.source.ids.dedup();
        }
    }
    roads
}

/// Clip segments against the actual tile union before joining. This also retains roads
/// crossing selected tiles whose original end nodes both lie outside the selection.
pub fn roads_on_tiles(ways:&[Way],to_world:impl Fn(f64,f64)->DVec2,tiles:&[crate::doc::TileKey])->Vec<Road>{
    let selected:std::collections::HashSet<_>=tiles.iter().copied().collect();
    let boxes:Vec<_>=tiles.iter().map(|k|{let lo=crate::doc::tile_origin(*k);(lo,lo+DVec2::splat(crate::doc::tile_size()))}).collect();
    let mut fresh=-1i64;let mut clipped=Vec::new();
    for way in ways{
        let mut run=way.clone();run.nodes.clear();run.geometry.clear();
        for i in 0..way.nodes.len().saturating_sub(1){
            let a=to_world(way.geometry[i].0,way.geometry[i].1);let b=to_world(way.geometry[i+1].0,way.geometry[i+1].1);let delta=b-a;
            if delta.length_squared()<1e-12{continue;}
            let mut intervals=Vec::new();
            for (lo,hi) in &boxes{
                let(mut first,mut last)=(0.0f64,1.0f64);let mut valid=true;
                for axis in 0..2{
                    if delta[axis].abs()<1e-12{if a[axis]<lo[axis]||a[axis]>hi[axis]{valid=false;break;}}
                    else{let x=(lo[axis]-a[axis])/delta[axis];let y=(hi[axis]-a[axis])/delta[axis];first=first.max(x.min(y));last=last.min(x.max(y));}
                }
                if valid&&last-first>1e-9{intervals.push((first,last));}
            }
            intervals.sort_by(|a,b|a.0.total_cmp(&b.0));let mut merged:Vec<(f64,f64)>=Vec::new();
            for (a,b) in intervals{if let Some(last)=merged.last_mut().filter(|last|a<=last.1+1e-9){last.1=last.1.max(b);}else{merged.push((a,b));}}
            if merged.is_empty(){if run.nodes.len()>1{clipped.push(run.clone());}run.nodes.clear();run.geometry.clear();}
            for (first,last) in merged{
                let mut start=a+delta*first;let mut end=a+delta*last;
                if !selected.contains(&crate::doc::tile_of(start)){start+=delta.normalize()*0.1;}
                if !selected.contains(&crate::doc::tile_of(end)){end-=delta.normalize()*0.1;}
                if !selected.contains(&crate::doc::tile_of(start))||!selected.contains(&crate::doc::tile_of(end)){continue;}
                let continuation=run.geometry.last().is_some_and(|p|(DVec2::new(p.0,p.1)-start).length()<1e-6);
                if !continuation{if run.nodes.len()>1{clipped.push(run.clone());}run.nodes.clear();run.geometry.clear();}
                if run.nodes.is_empty(){let node=if first<1e-9&&start==a{way.nodes[i]}else{let node=fresh;fresh-=1;node};run.nodes.push(node);run.geometry.push((start.x,start.y));}
                let node=if last>1.0-1e-9&&end==b{way.nodes[i+1]}else{let node=fresh;fresh-=1;node};run.nodes.push(node);run.geometry.push((end.x,end.y));
                if last<1.0-1e-9{if run.nodes.len()>1{clipped.push(run.clone());}run.nodes.clear();run.geometry.clear();}
            }
        }
        if run.nodes.len()>1{clipped.push(run);}
    }
    roads(&clipped,|x,y|DVec2::new(x,y),|_|true)
}

/// What a road is built as: its class, and whether it is one way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Kind {
    class: Class,
    oneway: bool,
    signature: u64,
    directional: bool,
}

/// The heading (degrees) from `a` to `b`.
fn heading(a: DVec2, b: DVec2) -> f64 {
    roads::heading_of(b - a)
}

/// How much the way turns going on from `in_dir` into `out_dir` (degrees, 0..180).
fn turn(in_dir: f64, out_dir: f64) -> f64 {
    (roads::norm_deg(out_dir - in_dir + 180.0) - 180.0).abs()
}

/// Ways of one class meeting end to end joined into one (where several end at a node, the
/// two that run on most nearly straight).
fn join(mut parts: Vec<(Kind, Vec<i64>, Vec<i64>)>, at: &HashMap<i64, DVec2>) -> Vec<(Kind, Vec<i64>, Vec<i64>)> {
    loop {
        // the ends at each node
        let mut ends: HashMap<i64, Vec<(usize, bool)>> = HashMap::new();
        for (i, p) in parts.iter().enumerate() {
            if p.1.first() == p.1.last() {
                continue;
            }
            ends.entry(p.1[0]).or_default().push((i, false));
            ends.entry(*p.1.last().unwrap()).or_default().push((i, true));
        }
        let mut best: Option<(f64, usize, bool, usize, bool)> = None;
        let mut nodes: Vec<&i64> = ends.keys().collect();
        nodes.sort();
        for n in nodes {
            let e = &ends[n];
            for a in 0..e.len() {
                for b in a + 1..e.len() {
                    let ((i, ie), (j, je)) = (e[a], e[b]);
                    if i == j || parts[i].0 != parts[j].0 || ((parts[i].0.oneway || parts[i].0.directional) && ie == je) {
                        continue;
                    }
                    // way i arriving at the node, way j leaving it
                    let dir_in = {
                        let p = &parts[i].1;
                        let (x, y) = if ie { (p[p.len() - 2], p[p.len() - 1]) } else { (p[1], p[0]) };
                        heading(at[&x], at[&y])
                    };
                    let dir_out = {
                        let p = &parts[j].1;
                        let (x, y) = if je { (p[p.len() - 1], p[p.len() - 2]) } else { (p[0], p[1]) };
                        heading(at[&x], at[&y])
                    };
                    let t = turn(dir_in, dir_out);
                    if t < 50.0 && best.map(|b| t < b.0).unwrap_or(true) {
                        best = Some((t, i, ie, j, je));
                    }
                }
            }
        }
        let Some((_, i, ie, j, je)) = best else { return parts };
        let mut a = parts[i].1.clone();
        if !ie {
            a.reverse();
        }
        let mut b = parts[j].1.clone();
        if je {
            b.reverse();
        }
        a.extend(b.into_iter().skip(1));
        // (the way kept runs as the first did, when it ran into the joint)
        if !ie {
            a.reverse();
        }
        let class = parts[i].0;
        let mut ids = parts[i].2.clone();
        ids.extend_from_slice(&parts[j].2); ids.sort_unstable(); ids.dedup();
        let (hi, lo) = (i.max(j), i.min(j));
        parts.remove(hi);
        parts.remove(lo);
        parts.push((class, a, ids));
    }
}

/// Built first: the roads others end on in their middle, then main roads, the long first.
fn order(parts: Vec<(Kind, Vec<i64>, Vec<i64>)>, at: &HashMap<i64, DVec2>) -> Vec<Road> {
    let n = parts.len();
    // where each node lies inside a road (not at its ends)
    let mut interior: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, p) in parts.iter().enumerate() {
        for node in &p.1[1..p.1.len() - 1] {
            interior.entry(*node).or_default().push(i);
        }
    }
    // before[j]: the roads j ends on
    let mut before: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (j, p) in parts.iter().enumerate() {
        for end in [p.1[0], *p.1.last().unwrap()] {
            for &i in interior.get(&end).into_iter().flatten() {
                if i != j && !before[j].contains(&i) {
                    before[j].push(i);
                }
            }
        }
    }
    let length = |p: &(Kind, Vec<i64>, Vec<i64>)| p.1.windows(2).map(|w| (at[&w[1]] - at[&w[0]]).length()).sum::<f64>();
    let rank: Vec<(Class, i64)> = parts.iter().map(|p| (p.0.class, -(length(p) as i64))).collect();
    let mut done = vec![false; n];
    let mut out: Vec<usize> = Vec::with_capacity(n);
    while out.len() < n {
        // the first in rank whose roads are all built (a ring of them: the first in rank)
        let ready = (0..n).filter(|&i| !done[i] && before[i].iter().all(|&b| done[b])).min_by_key(|&i| rank[i]);
        let next = ready.or_else(|| (0..n).filter(|&i| !done[i]).min_by_key(|&i| rank[i])).unwrap();
        done[next] = true;
        out.push(next);
    }
    out.into_iter()
        .map(|i| {
            let (kind, nodes, ids) = &parts[i];
            Road { class: kind.class, oneway: kind.oneway, points: nodes.iter().map(|n| at[n]).collect(), source: RoadSource { ids: ids.clone(), nodes: nodes.clone(), ..Default::default() }, asset: None }
        })
        .collect()
}

/// Points closer than this to the line through their neighbours are left out (m).
const TOLERANCE: f64 = 0.6;

/// A polyline with the points that add nothing left out (Douglas–Peucker).
pub fn simplify(p: &[DVec2]) -> Vec<DVec2> {
    fn dist(p: DVec2, a: DVec2, b: DVec2) -> f64 {
        let d = b - a;
        let l = d.length_squared();
        if l < 1e-12 {
            return (p - a).length();
        }
        let t = ((p - a).dot(d) / l).clamp(0.0, 1.0);
        (p - (a + d * t)).length()
    }
    fn keep(p: &[DVec2], lo: usize, hi: usize, out: &mut Vec<bool>) {
        if hi <= lo + 1 {
            return;
        }
        let (mut far, mut at) = (0.0, lo);
        for i in lo + 1..hi {
            let d = dist(p[i], p[lo], p[hi]);
            if d > far {
                far = d;
                at = i;
            }
        }
        if far > TOLERANCE {
            out[at] = true;
            keep(p, lo, at, out);
            keep(p, at, hi, out);
        }
    }
    if p.len() < 3 {
        return p.to_vec();
    }
    let mut k = vec![false; p.len()];
    k[0] = true;
    k[p.len() - 1] = true;
    keep(p, 0, p.len() - 1, &mut k);
    // (and no two points closer than 2 m: a way's nodes doubled at a tile's edge)
    let mut out: Vec<DVec2> = Vec::new();
    for (i, q) in p.iter().enumerate().filter(|(i, _)| k[*i]) {
        match out.last() {
            Some(last) if (*q - *last).length() < 2.0 => {
                // (the last point stays: the road ends where the way does)
                if i == p.len() - 1 && out.len() > 1 {
                    out.pop();
                    out.push(*q);
                }
            }
            _ => out.push(*q),
        }
    }
    out
}

/// The road's shape as straights and arcs. A slight bend (the points along a curve) is
/// rounded as widely as its legs allow, so the curve comes out as one; a corner with the
/// class's radius, or less where its legs are short (one too tight even so stays a corner).
pub fn segments(points: &[DVec2], class: Class) -> Vec<Seg> {
    let p = simplify(points);
    let mut out: Vec<Seg> = Vec::new();
    if p.len() < 2 {
        return out;
    }
    let mut from = p[0];
    for i in 1..p.len() - 1 {
        let (a, v, b) = (p[i - 1], p[i], p[i + 1]);
        let (h_in, h_out) = (heading(a, v), heading(v, b));
        let t = turn(h_in, h_out).to_radians();
        let room = 0.45 * (v - a).length().min((b - v).length());
        // (the room on the incoming leg left after the corner before it)
        let room = room.min(((v - from).length() - 0.01).max(0.0));
        let radius = if t < 35f64.to_radians() { 1000.0 } else { class.corner_radius() };
        let d = (radius * (t * 0.5).tan()).min(room);
        let arc = if t > 0.5f64.to_radians() && d > 0.3 {
            let t1 = v + roads::dir(h_in) * -d;
            let t2 = v + roads::dir(h_out) * d;
            roads::arc_to(t1, h_in, t2).map(|s| (t1, t2, s))
        } else {
            None
        };
        match arc {
            Some((t1, t2, s)) => {
                out.extend(roads::straight(from, t1));
                out.push(s);
                from = t2;
            }
            None => {
                out.extend(roads::straight(from, v));
                from = v;
            }
        }
    }
    out.extend(roads::straight(from, *p.last().unwrap()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn way(class: Class, nodes: &[i64], pts: &[(f64, f64)]) -> Way {
        // (the "latitude and longitude" here are world metres: the test's to_world is plain)
        Way { id: nodes.first().copied().unwrap_or(0), tags: Default::default(), reversed: false, restrictions: Vec::new(), class, oneway: false, nodes: nodes.to_vec(), geometry: pts.to_vec() }
    }

    #[test]
    fn a_street_in_two_ways_is_one_road_and_the_road_it_ends_on_comes_first() {
        let ways = vec![
            // a side street ending on the main road's middle node 2
            way(Class::Street, &[20, 2], &[(100.0, 200.0), (100.0, 100.0)]),
            // the main road from x 0 to 200 in two ways meeting at node 3
            way(Class::Main, &[1, 2, 3], &[(0.0, 100.0), (100.0, 100.0), (150.0, 100.0)]),
            way(Class::Main, &[3, 4], &[(150.0, 100.0), (200.0, 100.0)]),
        ];
        let r = roads(&ways, |a, b| DVec2::new(a, b), |_| true);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].class, Class::Main);
        assert_eq!(r[0].points.len(), 4);
        assert_eq!(r[1].class, Class::Street);
        // cut where it leaves the map, at the map's edge
        let r = roads(&ways, |a, b| DVec2::new(a, b), |p| p.x <= 160.0);
        assert_eq!(r[0].points.len(), 4);
        assert!((r[0].points[3].x - 159.5).abs() < 0.01, "{:?}", r[0].points);
    }

    #[test]
    fn a_corner_is_rounded_and_the_road_keeps_its_ends() {
        let pts = [DVec2::new(0.0, 0.0), DVec2::new(0.0, 100.0), DVec2::new(100.0, 100.0)];
        let s = segments(&pts, Class::Street);
        assert_eq!(s.len(), 3);
        assert!(s[1].radius > 19.0 && s[1].radius < 21.0, "{:?}", s[1]);
        let end = s.last().unwrap().end();
        assert!((end - pts[2]).length() < 1e-6);
        // each piece starts where the last ended, going on the same way
        for w in s.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-6);
            let h = w[0].curve().heading_at(w[0].length);
            assert!(turn(h, w[1].heading) < 0.01);
        }
        // a polyline along a gentle curve: one point of it per 10 m
        let bend: Vec<DVec2> = (0..=20).map(|i| DVec2::from_angle(i as f64 * 0.04) * 250.0).collect();
        let s = segments(&bend, Class::Main);
        let along: f64 = s.iter().map(|x| x.length).sum();
        assert!((along - 250.0 * 0.8).abs() < 2.0, "{along}");
    }

    #[test]
    fn crossing_ways_are_clipped_to_sparse_tile_selections() {
        let _map=crate::doc::one_map();omsi_map::set_tile_size(300.0);
        let mut w=way(Class::Street,&[1,2],&[(-100.0,150.0),(1000.0,150.0)]);w.id=42;
        let roads=roads_on_tiles(&[w],|x,y|DVec2::new(x,y),&[(0,0),(2,0)]);
        assert_eq!(roads.len(),2);
        for road in roads{assert_eq!(road.source.ids,vec![42]);assert!(road.points.iter().all(|p|[(0,0),(2,0)].contains(&crate::doc::tile_of(*p))));}
    }

    #[test]
    fn reverse_oneway_preserves_ids_tags_and_directional_meaning() {
        let body=r#"{"elements":[{"type":"way","id":42,"nodes":[1,2],"geometry":[{"lat":0,"lon":0},{"lat":0,"lon":100}],"tags":{"highway":"residential","oneway":"-1","lanes":"2","lanes:backward":"2","width":"6"}}]}"#;
        let ways=parse(body).unwrap();assert_eq!(ways[0].nodes,vec![2,1]);assert!(ways[0].reversed);
        let roads=roads(&ways,|a,b|DVec2::new(a,b),|_|true);assert_eq!(roads[0].source.ids,vec![42]);assert_eq!(roads[0].source.tag("oneway"),"-1");assert_eq!(roads[0].source.directional("lanes",true),"2");
        assert_eq!(roads[0].points[0],DVec2::new(0.0,100.0));
    }

    #[test]
    fn incompatible_widths_and_opposing_oneways_are_not_merged() {
        let mut a=way(Class::Street,&[1,2],&[(0.0,0.0),(100.0,0.0)]);
        let mut b=way(Class::Street,&[2,3],&[(100.0,0.0),(200.0,0.0)]);
        a.tags.insert("width".into(),"6".into());b.tags.insert("width".into(),"9".into());
        assert_eq!(roads(&[a.clone(),b.clone()],|x,y|DVec2::new(x,y),|_|true).len(),2);
        b.tags=a.tags.clone();a.oneway=true;b.oneway=true;b.nodes.reverse();b.geometry.reverse();
        assert_eq!(roads(&[a,b],|x,y|DVec2::new(x,y),|_|true).len(),2);
    }

    #[test]
    fn an_overpass_answer_is_read() {
        let body = r#"{"elements":[{"type":"way","id":7,"nodes":[1,2],"geometry":[{"lat":52.5,"lon":13.4},{"lat":52.501,"lon":13.4}],"tags":{"highway":"residential","name":"Hauptstr"}},{"type":"way","id":8,"nodes":[3,4],"geometry":[{"lat":52.5,"lon":13.4},{"lat":52.501,"lon":13.4}],"tags":{"highway":"footway"}}]}"#;
        let w = parse(body).unwrap();
        assert_eq!(w.len(), 1);
        let body = r#"{"elements":[{"type":"node","id":5,"lat":52.5,"lon":13.4,"tags":{"highway":"bus_stop","name":"Markt"}}]}"#;
        let s = parse_stops(body).unwrap();
        assert_eq!(s[0].name, "Markt");
        assert_eq!(w[0].class, Class::Street);
    }
}
