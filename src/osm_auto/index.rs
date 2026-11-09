//! CPU-only descriptors. The cache fingerprints model and texture dependencies as well
//! as the .sco/.sli, so changing an addon's mesh invalidates its measurements.

use crate::install::Roots;
use crate::library::{Asset, Category, Kind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoadAsset {
    pub rel: String,
    pub forward: u8,
    pub backward: u8,
    pub shared: u8,
    pub rail: u8,
    pub left_walk: bool,
    pub right_walk: bool,
    pub carriageway: f64,
    pub bounds: [f64; 2],
    pub surface: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildingAsset {
    pub rel: String,
    pub lo: [f64; 3],
    pub hi: [f64; 3],
    pub usage: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileStamp {
    pub path: PathBuf,
    len: u64,
    modified: u128,
}

fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        path: path.into(),
        len: meta.len(),
        modified: meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos(),
    })
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    files: Vec<FileStamp>,
    road: Option<RoadAsset>,
    building: Option<BuildingAsset>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    entries: BTreeMap<String, Entry>,
}

#[derive(Default)]
pub struct Index {
    pub roads: Vec<RoadAsset>,
    pub buildings: Vec<BuildingAsset>,
    pub rejected: usize,
    pub reused: usize,
    pub files: BTreeMap<String, Vec<FileStamp>>,
}

pub fn usage(text: &str) -> String {
    let text = text.to_lowercase();
    for (kind, words) in [
        ("industrial", &["industrial", "warehouse", "factory", "fabrik", "halle", "sklad"][..]),
        ("apartments", &["apartments", "apartment", "wohnblock", "hochhaus", "panel", "stamben"][..]),
        ("house", &["house", "haus", "detached", "bungalow", "cottage", "kuća", "kuca"][..]),
        ("retail", &["retail", "shop", "market", "laden", "commercial"][..]),
        ("civic", &["school", "hospital", "church", "schule", "kirche", "rathaus"][..]),
    ] {
        if words.iter().any(|w| text.contains(w)) {
            return kind.into();
        }
    }
    String::new()
}

fn building_hint(asset: &Asset) -> bool {
    !usage(&asset.haystack).is_empty()
        || ["building", "gebäude", "gebaeude", "zgrade", "edifice", "bâtiment"]
            .iter()
            .any(|s| asset.haystack.contains(s))
}

fn texture(path: &Path, roots: &Roots, name: &str) -> Option<PathBuf> {
    crate::assets::texture_dirs(&roots.omsi, &roots.content, path.parent()?)
        .iter()
        .map(|d| omsi_cfg::resolve_path(d, name))
        .find(|p| p.is_file())
}

fn road(path: &Path, rel: &str, roots: &Roots) -> Option<Entry> {
    let spline = omsi_scenery::Spline::load(path).ok()?;
    if spline.only_editor {
        return None;
    }
    let points: Vec<_> = spline.profiles.iter().flat_map(|p| &p.points).collect();
    if points.len() < 2 {
        return None;
    }
    let bounds = [
        points.iter().map(|p| p.x as f64).fold(f64::INFINITY, f64::min),
        points.iter().map(|p| p.x as f64).fold(f64::NEG_INFINITY, f64::max),
    ];
    if !bounds.iter().all(|n| n.is_finite()) || bounds[1] - bounds[0] < 1.0 {
        return None;
    }
    let mut desc = RoadAsset {
        rel: rel.into(),
        forward: 0,
        backward: 0,
        shared: 0,
        rail: 0,
        left_walk: false,
        right_walk: false,
        carriageway: 0.0,
        bounds,
        surface: String::new(),
    };
    let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &spline.paths {
        if !p.width.is_finite() || p.width <= 0.0 || !p.start[0].is_finite() {
            return None;
        }
        match p.kind {
            0 => {
                match p.direction {
                    0 => desc.forward = desc.forward.saturating_add(1),
                    1 => desc.backward = desc.backward.saturating_add(1),
                    2 => desc.shared = desc.shared.saturating_add(1),
                    _ => return None,
                }
                left = left.min((p.start[0] - p.width / 2.0) as f64);
                right = right.max((p.start[0] + p.width / 2.0) as f64);
            }
            1 => {
                if p.start[0] < 0.0 {
                    desc.left_walk = true;
                } else {
                    desc.right_walk = true;
                }
            }
            2 => desc.rail = desc.rail.saturating_add(1),
            _ => {}
        }
    }
    desc.carriageway = if left.is_finite() { right - left } else { 0.0 };
    if desc.carriageway > bounds[1] - bounds[0] + 1.0 {
        return None;
    }
    let mut files = vec![stamp(path)?];
    for tex in &spline.textures {
        if tex.file.trim().is_empty() {
            continue;
        }
        let file = texture(path, roots, &tex.file)?;
        files.push(stamp(&file)?);
        let hint = tex.file.to_lowercase();
        if hint.contains("asphalt") {
            desc.surface = "asphalt".into();
        }
        if hint.contains("pflaster") || hint.contains("cobble") {
            desc.surface = "paving_stones".into();
        }
        if hint.contains("gravel") || hint.contains("schotter") {
            desc.surface = "gravel".into();
        }
    }
    Some(Entry {
        files,
        road: Some(desc),
        building: None,
    })
}

fn building(path: &Path, asset: &Asset, roots: &Roots) -> Option<Entry> {
    let sco = omsi_scenery::SceneryObject::load(path).ok()?;
    // Scripted/conditional scenery and traffic objects need a user-authored descriptor.
    if sco.only_editor || sco.tree.is_some() || !sco.paths.is_empty() || sco.is_traffic_light || !sco.scripts.scripts.is_empty() {
        return None;
    }
    let mut files = vec![stamp(path)?];
    let (model, dir) = if let Some(ref file) = sco.model_file {
        let p = omsi_cfg::resolve_path(path.parent()?, file);
        files.push(stamp(&p)?);
        (omsi_model::Model::load(&p).ok()?, p.parent()?.to_path_buf())
    } else {
        (sco.model.clone(), path.parent()?.to_path_buf())
    };
    let defs = model.meshes.clone();
    if defs.is_empty() {
        return None;
    }
    for def in &defs {
        if def.visible.is_some() || !def.animations.is_empty() {
            return None;
        }
        let p = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&dir, "model"), &def.file);
        let p = if p.is_file() { p } else { omsi_cfg::resolve_path(&dir, &def.file) };
        files.push(stamp(&p)?);
        // Check every referenced mesh rather than accepting a partially loaded model.
        let mesh = omsi_o3d::load_mesh(&p).ok()?;
        for mat in &mesh.materials {
            if mat.texture.trim().is_empty() {
                continue;
            }
            let tex = crate::assets::texture_dirs(&roots.omsi, &roots.content, &dir)
                .iter()
                .map(|d| omsi_cfg::resolve_path(d, &mat.texture))
                .find(|p| p.is_file())?;
            files.push(stamp(&tex)?);
        }
    }
    let measured = crate::assets::load_object(path, &roots.omsi, &roots.content)?;
    if measured.technical || measured.abs_height || measured.parts.iter().any(|p| p.visible.is_some() || p.faded.is_some()) {
        return None;
    }
    let lo = measured.lo.as_dvec3();
    let hi = measured.hi.as_dvec3();
    let size = hi - lo;
    if !lo.is_finite() || !hi.is_finite() || size.x < 2.0 || size.y < 2.0 || size.z < 2.5 {
        return None;
    }
    Some(Entry {
        files,
        road: None,
        building: Some(BuildingAsset {
            rel: asset.rel.clone(),
            lo: lo.to_array(),
            hi: hi.to_array(),
            usage: usage(&asset.haystack),
        }),
    })
}

pub fn build(
    assets: &[Asset],
    roots: &Roots,
    dirs: &[String],
    check: &impl Fn() -> Result<(), String>,
    progress: &impl Fn(usize, usize, &str),
) -> Result<Index, String> {
    let cache_path = roots.content.join("openMapper").join("osm_asset_index_v1.json");
    let mut cache: Cache = std::fs::read(&cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|c: &Cache| c.version == 1)
        .unwrap_or_default();
    cache.version = 1;
    let mut result = Index::default();
    let mut assets: Vec<_> = assets
        .iter()
        .filter(|a| {
            (a.kind == Kind::Spline && matches!(a.category, Category::Roads | Category::Tracks))
                || (a.kind == Kind::Object
                    && a.category == Category::Objects
                    && if dirs.is_empty() {
                        building_hint(a)
                    } else {
                        dirs.iter().any(|d| a.dir.eq_ignore_ascii_case(d))
                    })
        })
        .collect();
    assets.sort_by(|a, b| a.rel.cmp(&b.rel));
    let mut stamps: HashMap<PathBuf, Option<FileStamp>> = HashMap::new();
    for (i, asset) in assets.iter().enumerate() {
        check()?;
        progress(i, assets.len(), &asset.rel);
        let path = roots.resolve(&asset.rel);
        let key = format!("{}|{}", path.display(), asset.haystack);
        let cached = cache.entries.get(&key).filter(|e| {
            !e.files.is_empty()
                && e.files
                    .iter()
                    .all(|s| stamps.entry(s.path.clone()).or_insert_with(|| stamp(&s.path)).as_ref() == Some(s))
        });
        let entry = if let Some(cached) = cached {
            result.reused += 1;
            Some(cached.clone())
        } else if asset.kind == Kind::Spline {
            road(&path, &asset.rel, roots)
        } else {
            building(&path, asset, roots)
        };
        if let Some(entry) = entry {
            result.files.insert(asset.rel.clone(), entry.files.clone());
            result.roads.extend(entry.road.clone());
            result.buildings.extend(entry.building.clone());
            cache.entries.insert(key, entry);
        } else {
            cache.entries.remove(&key);
            result.rejected += 1;
        }
    }
    check()?;
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        let _ = crate::doc::safe::write_atomic(&cache_path, &bytes);
    }
    Ok(result)
}

pub fn unchanged(files: &[FileStamp]) -> bool {
    files.iter().all(|file| stamp(&file.path).as_ref() == Some(file))
}
