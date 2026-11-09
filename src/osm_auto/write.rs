use super::{Plan, RoadOption, buildings::Placement};
use crate::{
    doc::{Document, tile_of},
    install::Roots,
    osm::Road,
};
use openmapper_mesh::{Material, Model, Part};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub struct Prepared {
    pub roads: Vec<Road>,
    pub buildings: Vec<Placement>,
    pub files: BTreeMap<PathBuf, Option<Vec<u8>>>,
    pub warnings: Vec<String>,
}

fn track_new(path: &Path, files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
    if !path.exists() {
        files.entry(path.into()).or_insert(None);
    }
}

fn atlas(path: &Path, wall: bool, files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    track_new(path, files);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let image = image::RgbImage::from_fn(64, 64, |x, y| {
        let color = if wall {
            if (12..28).contains(&(x % 32)) && (10..40).contains(&y) {
                [70, 92, 105]
            } else if y > 59 {
                [120, 115, 108]
            } else {
                [192, 179, 157]
            }
        } else {
            let n = ((x.wrapping_mul(7919) ^ y.wrapping_mul(104729)) % 9) as u8;
            [72 + n, 73 + n, 74 + n]
        };
        image::Rgb(color)
    });
    image.save(path).map_err(|e| e.to_string())
}

pub(super) fn validate_plan(plan: &Plan, doc: &Document) -> Result<(), String> {
    if plan.revision != super::revision(doc) {
        return Err("Document changed since this preview; replan before building".into());
    }
    if !super::index::unchanged(&plan.dependencies) {
        return Err("Installed assets changed since this preview; replan before building".into());
    }
    let mut occupied = super::occupancy::Occupancy::from_polygons(plan.base_occupied.iter().cloned());
    for choice in &plan.roads {
        let option = choice.candidates.get(choice.selected).ok_or("Invalid road choice")?;
        occupied.extend(super::road_occupancy(&choice.road, option.half_width()));
    }
    occupied.extend(super::junction_occupancy(&plan.roads));
    for p in &plan.buildings {
        if occupied.conflicts(&p.footprint, &p.source.holes) {
            return Err(format!("Building {} conflicts with the selected road assets; preview again", p.source.id));
        }
        occupied.insert(p.footprint.clone());
    }
    Ok(())
}

pub fn prepare(plan: &Plan, roots: &Roots, doc: &Document) -> Result<Prepared, String> {
    validate_plan(plan, doc)?;
    let mut files = BTreeMap::new();
    let result = (|| {
        let mut roads = Vec::new();
        let mut buildings = Vec::new();
        let mut warnings = Vec::new();
        let map_hash = crate::geo::fetch::fnv(doc.dir.to_string_lossy().as_bytes());
        for choice in &plan.roads {
            let selected = choice.candidates.get(choice.selected).ok_or("Invalid road choice")?;
            let rel = match selected {
                RoadOption::Installed(asset) => {
                    if !roots.resolve(&asset.rel).is_file() {
                        return Err(format!("Asset disappeared: {}", asset.rel));
                    }
                    asset.rel.clone()
                }
                RoadOption::Generated(target) => {
                    let mut spec = super::roads::spec(target);
                    spec.name = format!("{}_m{map_hash:016x}", spec.name);
                    let tex = roots.content.join("Splines/openMapper/texture");
                    let pavement = tex.join("osm_pavement_v1.png");
                    let surface = match target.surface.as_str() {
                        "gravel" | "fine_gravel" => "gravel",
                        "unpaved" | "ground" | "dirt" | "mud" | "sand" => "earth",
                        "paving_stones" | "cobblestone" | "sett" => "paving",
                        "concrete" => "concrete",
                        _ => "asphalt",
                    };
                    let roadway = tex.join(format!("osm_{surface}_v1.png"));
                    if !roadway.exists() {
                        track_new(&roadway, &mut files);
                        std::fs::create_dir_all(&tex).map_err(|e| e.to_string())?;
                        let base = match surface {
                            "gravel" => [140u8, 133, 115],
                            "earth" => [128, 102, 73],
                            "paving" => [130, 126, 119],
                            "concrete" => [155, 154, 147],
                            _ => [72, 73, 74],
                        };
                        image::RgbImage::from_fn(64, 64, |x, y| {
                            let n = if surface == "paving" && (x % 16 == 0 || y % 16 == 0) {
                                -15
                            } else {
                                ((x.wrapping_mul(7919) ^ y.wrapping_mul(104729)) % 9) as i16
                            };
                            image::Rgb(base.map(|v| (v as i16 + n).clamp(0, 255) as u8))
                        })
                        .save(&roadway)
                        .map_err(|e| e.to_string())?;
                    }
                    if !pavement.exists() {
                        track_new(&pavement, &mut files);
                        image::RgbImage::from_pixel(64, 64, image::Rgb([155, 151, 142]))
                            .save(&pavement)
                            .map_err(|e| e.to_string())?;
                    }
                    spec.roadway_tex = roadway;
                    spec.footway_tex = pavement;
                    let stem = spec.file_stem();
                    let folder = roots.content.join("Splines/openMapper");
                    let sli = folder.join(format!("{stem}.sli"));
                    let source = folder.join(format!("{stem}.road.json"));
                    if sli.exists() {
                        let text = omsi_cfg::decode_text(&std::fs::read(&sli).map_err(|e| e.to_string())?);
                        if text.replace("\r\n", "\n") != spec.sli() {
                            return Err(format!("Generated road was edited; preserve or rename it before replanning: {}", sli.display()));
                        }
                    } else {
                        if source.exists() {
                            return Err(format!(
                                "Generated road source exists without its spline; review it before regenerating: {}",
                                source.display()
                            ));
                        }
                        track_new(&sli, &mut files);
                        track_new(&source, &mut files);
                        if spec.marked {
                            track_new(&tex.join(spec.roadway_tex_name()), &mut files);
                        }
                        spec.write(&roots.content).map_err(|e| e.to_string())?;
                    }
                    let definition = omsi_scenery::Spline::load(&sli).map_err(|e| e.to_string())?;
                    for texture in &definition.textures {
                        if !omsi_cfg::resolve_path(&tex, &texture.file).is_file() {
                            return Err(format!("Generated road texture is missing: {}", texture.file));
                        }
                    }
                    format!("Splines\\openMapper\\{stem}.sli")
                }
            };
            let mut road = choice.road.clone();
            road.asset = Some(rel);
            roads.push(road);
        }
        for placement in &plan.buildings {
            let mut p = placement.clone();
            // A level foundation on the highest sampled corner, with a bounded cut/fill.
            let heights: Vec<_> = p.footprint.iter().map(|p| doc.ground(*p)).collect();
            let low = heights.iter().copied().fold(f64::INFINITY, f64::min);
            let high = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if high - low > 2.0 {
                let warning = format!("Building {} skipped: needs a terrain/foundation review ({:.2} m height variation)", p.source.id, high - low);
                log::warn!("OSM import: {warning}");
                warnings.push(warning);
                continue;
            }
            p.base += high - doc.ground(p.at);
            if p.asset.is_none() {
                let mesh = super::buildings::mesh(&p.source, p.at)?;
                let hash = crate::geo::fetch::fnv(&serde_json::to_vec(&(&p.source, map_hash)).map_err(|e| e.to_string())?);
                let stem = format!("osm_{hash:016x}");
                let rel = format!("Sceneryobjects\\openMapper\\OSM\\{stem}\\{stem}.sco");
                let folder = roots.content.join("Sceneryobjects/openMapper/OSM").join(&stem);
                let shared = roots.content.join("Sceneryobjects/openMapper/OSM/texture");
                atlas(&shared.join("osm_wall_v1.png"), true, &mut files)?;
                atlas(&shared.join("osm_roof_v1.png"), false, &mut files)?;
                let source = crate::editable::Source {
                    name: format!("OSM building {}", p.source.id),
                    model: Model {
                        parts: vec![Part::new("shell", mesh)],
                        materials: vec![
                            Material {
                                texture: Some("..\\..\\texture\\osm_wall_v1.png".into()),
                                ..Material::colored("walls", [1.0; 4])
                            },
                            Material {
                                texture: Some("..\\..\\texture\\osm_roof_v1.png".into()),
                                ..Material::colored("roof", [1.0; 4])
                            },
                        ],
                    },
                    collision: true,
                    ..Default::default()
                };
                if !folder.join(format!("{stem}.sco")).exists() {
                    if folder.join(format!("{stem}.openmapper.json")).exists()
                        || folder.join("model").read_dir().ok().is_some_and(|mut files| files.next().is_some())
                    {
                        return Err(format!("Partial generated building assets already exist; review {}", folder.display()));
                    }
                    for file in [
                        format!("{stem}.sco"),
                        format!("{stem}.openmapper.json"),
                        format!("model/{stem}_shell.x"),
                        format!("model/{stem}_collision.x"),
                    ] {
                        track_new(&folder.join(file), &mut files);
                    }
                    crate::editable::save(roots, &rel, &source).map_err(|e| e.to_string())?;
                }
                let loaded = crate::assets::load_object(&roots.resolve(&rel), &roots.omsi, &roots.content);
                if loaded.as_ref().is_none_or(|model| {
                    model
                        .parts
                        .iter()
                        .flat_map(|p| &p.materials)
                        .any(|m| m.texture.as_ref().is_none_or(|p| !p.is_file()))
                }) {
                    return Err(format!("Generated building failed validation: {}", p.source.id));
                }
                p.asset = Some(rel);
            }
            if !doc.tiles.contains_key(&tile_of(p.at)) {
                return Err("Document tiles changed during planning".into());
            }
            buildings.push(p);
        }
        let manifest = doc.dir.join("openMapper/osm_plan.json");
        files.insert(manifest.clone(), std::fs::read(&manifest).ok());
        let report_warnings: Vec<_> = plan.warnings.iter().chain(warnings.iter()).collect();
        let report = serde_json::json!({"version":1,"source":"OpenStreetMap contributors (ODbL)","plan":plan.key,"roads":plan.roads.iter().map(|r|serde_json::json!({"osm_ids":r.road.source.ids,"tags":r.road.source.tags,"choice":r.candidates[r.selected].label(),"reason":r.reason})).collect::<Vec<_>>(),"buildings":buildings,"warnings":report_warnings});
        std::fs::create_dir_all(manifest.parent().unwrap()).map_err(|e| e.to_string())?;
        crate::doc::safe::write_atomic(&manifest, &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(Prepared {
            roads,
            buildings,
            files: files.clone(),
            warnings,
        })
    })();
    if result.is_err() {
        for (path, before) in &files {
            match before {
                Some(bytes) => {
                    let _ = crate::doc::safe::write_atomic(path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_shared_road_has_a_bidirectional_path_and_resolved_textures() {
        let dir = std::env::temp_dir().join(format!("osm-write-{}", std::process::id()));
        let roots = Roots {
            omsi: dir.join("omsi"),
            content: dir.join("content"),
        };
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = dir.join("map");
        let target = super::super::roads::Target {
            forward: 1,
            backward: 0,
            shared: true,
            width: 3.5,
            left_walk: 0.0,
            right_walk: 0.0,
            surface: "asphalt".into(),
            inferred: false,
        };
        let road = Road {
            class: crate::osm::Class::Street,
            oneway: false,
            points: vec![glam::DVec2::new(10.0, 10.0), glam::DVec2::new(10.0, 90.0)],
            source: Default::default(),
            asset: None,
        };
        let plan = Plan {
            key: "fixture".into(),
            revision: super::super::revision(&doc),
            roads: vec![super::super::RoadChoice {
                road,
                candidates: vec![RoadOption::Generated(target)],
                selected: 0,
                reason: String::new(),
            }],
            buildings: vec![],
            warnings: vec![],
            catalogue: (0, 0, 0, 0),
            base_occupied: vec![],
            dependencies: vec![],
            skipped_roads: 0,
            skipped_buildings: 0,
        };
        let prepared = prepare(&plan, &roots, &doc).unwrap();
        let rel = prepared.roads[0].asset.as_ref().unwrap();
        let spline = omsi_scenery::Spline::load(&roots.resolve(rel)).unwrap();
        assert!(spline.paths.iter().any(|p| p.kind == 0 && p.direction == 2));
        assert_eq!(spline.paths.iter().filter(|p| p.kind == 0).count(), 1);
        assert!(crate::assets::load_object(&roots.content.join("missing.sco"), &roots.omsi, &roots.content).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
