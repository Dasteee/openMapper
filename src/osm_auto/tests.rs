use super::*;
use crate::library::{Category, Kind};
use crate::osm_auto::buildings::Footprint;

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<DVec2> {
    vec![DVec2::new(x, y), DVec2::new(x + w, y), DVec2::new(x + w, y + h), DVec2::new(x, y + h)]
}
fn footprint(id: &str, x: f64, y: f64) -> Footprint {
    Footprint {
        id: id.into(),
        tags: Default::default(),
        outer: rectangle(x, y, 20.0, 10.0),
        holes: vec![],
        height: 9.0,
        min_height: 0.0,
        known: true,
        usage: "house".into(),
        issue: None,
    }
}

fn fixture(name: &str) -> (std::path::PathBuf, Roots, Vec<Asset>) {
    let dir = std::env::temp_dir().join(format!("osm-auto-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let roots = Roots {
        omsi: dir.join("omsi"),
        content: dir.join("content"),
    };
    let roadtex = dir.join("asphalt.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([80, 80, 80])).save(&roadtex).unwrap();
    let spec = crate::road_maker::RoadSpec {
        name: "fixture_street".into(),
        lane_width: 3.0,
        footway: 0.0,
        kerb: 0.0,
        roadway_tex: roadtex,
        ..Default::default()
    };
    let rel = spec.write(&roots.content).unwrap();
    let mut lib = crate::library::roads_for_tests(&[(&rel, 3.0)]);
    let f = footprint("fixture", 0.0, 0.0);
    let mesh = buildings::mesh(&f, DVec2::ZERO).unwrap();
    let source = crate::editable::Source {
        name: "Fixture house".into(),
        model: openmapper_mesh::Model {
            parts: vec![openmapper_mesh::Part::new("shell", mesh)],
            materials: vec![openmapper_mesh::Material::default(), openmapper_mesh::Material::default()],
        },
        ..Default::default()
    };
    let rel = "Sceneryobjects\\Fixture\\house.sco";
    crate::editable::save(&roots, rel, &source).unwrap();
    let mut house = lib.assets[0].clone();
    house.kind = Kind::Object;
    house.category = Category::Objects;
    house.rel = rel.into();
    house.haystack = "house building".into();
    house.dir = "Fixture".into();
    lib.assets.push(house);
    (dir, roots, lib.assets)
}

#[test]
fn catalogue_reuses_descriptors_and_invalidates_changed_mesh_dependencies() {
    let (dir, roots, assets) = fixture("cache");
    let first = index::build(&assets, &roots, &[], &|| Ok(()), &|_, _, _| {}).unwrap();
    assert_eq!((first.roads.len(), first.buildings.len(), first.reused), (1, 1, 0));
    let second = index::build(&assets, &roots, &[], &|| Ok(()), &|_, _, _| {}).unwrap();
    assert_eq!(second.reused, 2);
    let mesh_path = roots.content.join("Sceneryobjects/Fixture/model/house_shell.x");
    let mut bytes = std::fs::read(&mesh_path).unwrap();
    bytes.extend_from_slice(b"\n// changed mesh dependency\n");
    std::fs::write(&mesh_path, bytes).unwrap();
    let third = index::build(&assets, &roots, &[], &|| Ok(()), &|_, _, _| {}).unwrap();
    assert_eq!(third.reused, 1);
    assert_eq!(third.buildings.len(), 1);
    std::fs::remove_file(mesh_path).unwrap();
    let broken = index::build(&assets, &roots, &[], &|| Ok(()), &|_, _, _| {}).unwrap();
    assert!(broken.buildings.is_empty());
    assert_eq!(broken.rejected, 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn installed_and_generated_assets_apply_together_and_undo_redo_includes_files() {
    let (dir, roots, assets) = fixture("apply");
    let mut doc = crate::selection::tests::flat_map(2);
    doc.dir = dir.join("map");
    // A steep second tile must not prevent importing the roads and flat-tile buildings.
    let terrain = &mut doc.tiles.get_mut(&(1, 0)).unwrap().terrain;
    let side = (terrain.heights.len() as f64).sqrt() as usize;
    let cell = crate::doc::tile_size() / (side - 1) as f64;
    for (i, height) in terrain.heights.iter_mut().enumerate() {
        *height = ((i % side) as f64 * cell * 0.25) as f32;
    }
    let road = |id: i64, x: f64, lanes: &str, width: &str| Road {
        class: Class::Street,
        oneway: false,
        points: vec![DVec2::new(x, 20.0), DVec2::new(x, 80.0)],
        asset: None,
        source: crate::osm::RoadSource {
            ids: vec![id],
            tags: [("lanes".into(), lanes.into()), ("width".into(), width.into())].into_iter().collect(),
            ..Default::default()
        },
    };
    let house = footprint("way/100", 100.0, 100.0);
    let mut court = footprint("relation/200", 200.0, 200.0);
    court.outer = rectangle(200.0, 200.0, 20.0, 20.0);
    court.holes.push(rectangle(205.0, 205.0, 10.0, 10.0));
    let spec = Spec {
        key: "fixture".into(),
        revision: revision(&doc),
        roots: roots.clone(),
        assets,
        roads: vec![road(1, 20.0, "2", "6"), road(2, 40.0, "1", "3.5")],
        buildings: vec![house, court, footprint("way/steep", 400.0, 100.0)],
        overrides: Default::default(),
        segment_overrides: Default::default(),
        building_dirs: vec![],
        procedural: true,
        existing_roads: vec![],
        existing_objects: vec![],
        tiles: vec![(0, 0), (1, 0)],
    };
    let plan = plan(spec, &|| Ok(()), &|_, _, _, _| {}).unwrap();
    assert_eq!((plan.roads.len(), plan.buildings.len(), plan.warnings.len()), (2, 3, 0));
    assert!(matches!(plan.roads[0].candidates[plan.roads[0].selected], RoadOption::Installed(_)));
    assert!(matches!(plan.roads[1].candidates[plan.roads[1].selected], RoadOption::Generated(_)));
    assert_eq!(plan.buildings.iter().filter(|b| b.asset.is_some()).count(), 2);
    let prepared = prepare(&plan, &roots, &doc).unwrap();
    assert_eq!((prepared.roads.len(), prepared.buildings.len(), prepared.warnings.len()), (2, 2, 1));
    assert!(prepared.warnings[0].contains("way/steep"));
    assert!(prepared.buildings.iter().all(|p| p.source.id != "way/steep"));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(doc.dir.join("openMapper/osm_plan.json")).unwrap()).unwrap();
    assert!(report["warnings"][0].as_str().unwrap().contains("way/steep"));
    let generated: Vec<_> = prepared.files.iter().filter(|(_, before)| before.is_none()).map(|(p, _)| p.clone()).collect();
    assert!(!generated.is_empty());
    let mut editor = crate::editor::Editor::default();
    editor.content_dir = Some(roots.content.clone());
    editor.open(doc);
    let mut models = crate::assets::Assets::new(roots.omsi.clone(), roots.content.clone());
    let mark = editor.history.mark();
    editor.history.push_snapshot(
        "OSM",
        crate::history::Snapshot {
            files: prepared.files,
            ..Default::default()
        },
    );
    let (built, _) = editor.build_osm(&mut models, &prepared.roads, &[], None);
    assert_eq!(built, 2);
    assert_eq!(editor.place_osm_planned_buildings(&prepared.buildings), 2);
    editor.history.merge_since(mark, "OSM");
    editor.undo();
    assert!(
        editor
            .doc
            .as_ref()
            .unwrap()
            .tiles
            .values()
            .all(|t| t.tile.objects.is_empty() && t.tile.splines.is_empty())
    );
    assert!(generated.iter().all(|p| !p.exists()));
    editor.redo();
    assert!(generated.iter().all(|p| p.exists()));
    let doc = editor.doc.as_ref().unwrap();
    assert_eq!(doc.tiles[&(0, 0)].tile.objects.len(), 2);
    for building in &prepared.buildings {
        assert!(crate::assets::load_object(&roots.resolve(building.asset.as_ref().unwrap()), &roots.omsi, &roots.content).is_some());
    }
    drop(models);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn building_progress_counts_examined_items_including_rejections() {
    let doc = crate::selection::tests::flat_map(1);
    let mut bad = footprint("way/1", 30.0, 30.0);
    bad.issue = Some("fixture rejection".into());
    let spec = Spec {
        key: "progress".into(),
        revision: revision(&doc),
        roots: Roots {
            omsi: Default::default(),
            content: Default::default(),
        },
        assets: vec![],
        roads: vec![],
        buildings: vec![bad, footprint("way/2", 60.0, 60.0), footprint("way/3", 1000.0, 1000.0)],
        overrides: Default::default(),
        segment_overrides: Default::default(),
        building_dirs: vec![],
        procedural: true,
        existing_roads: vec![],
        existing_objects: vec![],
        tiles: vec![(0, 0)],
    };
    let events = std::cell::RefCell::new(Vec::new());
    let result = plan_with_index(spec, index::Index::default(), &|| Ok(()), &|phase, i, total, _| {
        if phase == Phase::Buildings {
            events.borrow_mut().push((i, total));
        }
    })
    .unwrap();
    assert_eq!(*events.borrow(), vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
    assert_eq!(result.buildings.len(), 1);
    assert_eq!(result.skipped_buildings, 2);
}

#[test]
#[ignore = "Explicit read-only replay of local cached OSM and descriptor files"]
fn replay_cached_preview() {
    let setting = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"));
    let map = std::path::PathBuf::from(setting("OPENMAPPER_REPLAY_MAP"));
    let content = std::path::PathBuf::from(setting("OPENMAPPER_REPLAY_CONTENT"));
    let roots = Roots {
        omsi: content.join("unused-install"),
        content: content.clone(),
    };
    let doc = crate::doc::Document::load(&roots, &map.join("global.cfg"), |_| {}).unwrap();
    let geo = crate::geo::GeoFile::load(&map)
        .georef(omsi_map::world_coordinates(), crate::doc::tile_size())
        .unwrap();
    let raw = std::fs::read_to_string(setting("OPENMAPPER_REPLAY_BUILDINGS")).unwrap();
    let parsed = crate::osm_buildings::parse(&raw).unwrap();
    let buildings: Vec<_> = parsed
        .iter()
        .map(|b| Footprint {
            id: b.id.clone(),
            tags: b.tags.clone(),
            outer: b.outline.iter().map(|p| geo.world(p.0, p.1)).collect(),
            holes: b.holes.iter().map(|h| h.iter().map(|p| geo.world(p.0, p.1)).collect()).collect(),
            height: b.height,
            min_height: b.min_height,
            known: b.known,
            usage: index::usage(b.tags.get("building").or_else(|| b.tags.get("building:part")).map(String::as_str).unwrap_or("")),
            issue: b.issue.clone(),
        })
        .collect();
    let ways = crate::osm::parse(&std::fs::read_to_string(setting("OPENMAPPER_REPLAY_ROADS")).unwrap()).unwrap();
    let roads = crate::osm::roads_on_tiles(&ways, |a, b| geo.world(a, b), &doc.tiles.keys().copied().collect::<Vec<_>>());
    let cache: serde_json::Value = serde_json::from_slice(&std::fs::read(content.join("openMapper/osm_asset_index_v1.json")).unwrap()).unwrap();
    let mut index = index::Index::default();
    for entry in cache["entries"].as_object().unwrap().values() {
        if !entry["road"].is_null() {
            let asset: index::RoadAsset = serde_json::from_value(entry["road"].clone()).unwrap();
            index.files.insert(asset.rel.clone(), serde_json::from_value(entry["files"].clone()).unwrap());
            index.roads.push(asset);
        }
        if !entry["building"].is_null() {
            let asset: index::BuildingAsset = serde_json::from_value(entry["building"].clone()).unwrap();
            index.files.insert(asset.rel.clone(), serde_json::from_value(entry["files"].clone()).unwrap());
            index.buildings.push(asset);
        }
    }
    index.roads.sort_by(|a, b| a.rel.cmp(&b.rel));
    index.buildings.sort_by(|a, b| a.rel.cmp(&b.rel));
    let (existing_roads, existing_objects) = existing(&doc, &crate::library::Library::default());
    eprintln!(
        "REPLAY INPUT: {} source buildings, {} prepared roads, {} building models, {} road assets, {} tiles",
        buildings.len(),
        roads.len(),
        index.buildings.len(),
        index.roads.len(),
        doc.tiles.len()
    );
    let spec = Spec {
        key: "read-only-replay".into(),
        revision: revision(&doc),
        roots,
        assets: vec![],
        roads,
        buildings,
        overrides: Default::default(),
        segment_overrides: Default::default(),
        building_dirs: vec![],
        procedural: true,
        existing_roads,
        existing_objects,
        tiles: doc.tiles.keys().copied().collect(),
    };
    let started = std::time::Instant::now();
    let phase = std::cell::Cell::new(Phase::Roads);
    let result = plan_with_index(
        spec,
        index,
        &|| {
            if started.elapsed() > std::time::Duration::from_secs(120) {
                Err("Replay exceeded 120-second planning budget".into())
            } else {
                Ok(())
            }
        },
        &|stage, done, total, _| {
            if stage != phase.get() {
                phase.set(stage);
                eprintln!("REPLAY STAGE: {stage:?}, total {total}, elapsed {:.3}s", started.elapsed().as_secs_f64());
            }
            if stage == Phase::Buildings && (done % 1000 == 0 || done == total) {
                eprintln!("REPLAY BUILDINGS: {done}/{total}, elapsed {:.3}s", started.elapsed().as_secs_f64());
            }
        },
    )
    .unwrap();
    eprintln!(
        "REPLAY RESULT: {:.3}s planning; {} roads, {} accepted buildings, {} skipped buildings",
        started.elapsed().as_secs_f64(),
        result.roads.len(),
        result.buildings.len(),
        result.skipped_buildings
    );
    super::write::validate_plan(&result, &doc).expect("A completed preview must pass read-only build preflight");
}
