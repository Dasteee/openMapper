# openMapper: road to 1.0

openMapper 0.1 already opens, edits and saves OMSI 2 maps (roads with automatic junctions,
objects, terrain, painting, tiles, game logic, import, test drive). What keeps it from being
a complete editor, and who works on what.

## Where it stands (October 2026)

Done: every part of an OMSI 2 map can be made and changed in openMapper without OMSI 2's
editor: tiles, ground (brushes, ramps, height maps, painting by brush, rule, along roads),
water, roads with junctions, lights and crossings, tracks with switches, objects, plants
(brush and areas), game logic (stops in pairs, start points, car parks, AI paths), lines,
trips, tours, traffic, calendar, signal routes, chrono scenarios, descriptions; real places
from satellite imagery, terrain heights, OpenStreetMap and GPS tracks; tiles from other
maps; checks, safe saving, sharing (zip and plan), a test drive, a written tutorial and a
command search.

Still open for 1.0:
* Releases: `tools/package-macos.sh`, `tools/package-linux.sh` and `tools/package-windows.ps1`
  package the program, and `.github/workflows/release.yml` builds Windows, macOS and Linux for
  x64 and ARM64 on a `v*` tag and publishes them with the notes of that version from
  `CHANGELOG.md`. 0.1.0 is the first one. Still wanted: signing and notarising the macOS app and
  a signed Windows build.
* A pass in openOMSI over bigger maps made only with openMapper (an OSM town): Map > Check in
  openOMSI runs the game's own checks; the sample with a road built across it passes them
  (0 of 34 chained ends apart, 21 of 21 trip joints joined, no buried road). Bad Belzig from
  OpenStreetMap on 9 tiles (44 roads, 31 junctions, 16 stops, 1753 trees; October 2026): 0 of
  425 chained ends apart, no buried road, 0 of 17794 wheel points under a face; the 25 of 4421
  lane points without road under them all lie at the map's edge or a street's dead end.
* The interface's texts read through once more by a native speaker in each language.

## Workstreams

Each stream has its own branch and owns its files. Shared files (`src/ui.rs`,
`src/editor.rs`, `src/app.rs`, `src/main.rs`, `src/i18n.rs`, `src/icons.rs`) are touched
only with small hooks: a new menu item, a new `Action`, a call into the stream's module. Big
pieces of code go into new files.

| Stream | Branch | Owns | Goal |
| --- | --- | --- | --- |
| A. Timetable | `feature/timetable` | `src/timetable/` (new), `src/ui_timetable.rs` (new) | Lines, trips and tours (`TTData`): stops picked from the map, the route found along the splines between them, times; written in OMSI 2's formats so the map can be driven in service, not only freely |
| B. Editing | `feature/editing` | `src/editor.rs` (selection, drag, spline edit), `src/clipboard.rs` (new) | Multi-select (Shift-click, box), copy / paste / duplicate, numeric position and height, moving road ends and reshaping a road after it is drawn, snapping |
| C. Quality | `feature/quality` | `src/render/`, `src/doc/`, `src/check.rs` (new) | Map check (missing files, unlinked splines, stops off a path, start points off a road), speed on big maps (Berlin-Spandau), safe saving (backup, atomic writes), tests |
| D. Integration | `main` | `README.md`, `docs/`, merging | Merges the streams, keeps the README true, overview map, release polish |

## Rules for everyone

* Every user-facing text goes through `tr("English", "Русский")`.
* Follow the code around you: short comments that say why, no dead code.
* Commit often on your branch with plain messages. **No AI mentions anywhere**: no
  `Co-Authored-By` trailers, no "Generated with", nothing in code comments.
* `cargo build --release` and `cargo test --release` pass before every commit.
* Never write into the OMSI 2 installation; maps and generated content go into openOMSI's
  content folder.
* Check visually with `OPENMAPPER_SHOT=out.png ./target/release/openmapper path/to/global.cfg`.
* When you add a shared hook (new `Action`, new menu entry), note it in the log below so the
  merge is easy.

## Hooks log

Add a line when you touch a shared file: stream, file, what.
* D · `src/ui.rs`: `Action::LookAt(DVec2)`, `UiState::minimap`, `Status::cam`, View menu "Overview map", minimap drawn in the central panel; welcome list maps a recent installation map to its content copy.
* D · `src/app.rs`: `Status::cam` filled, `Action::LookAt` handled, key `M` toggles the overview map.
* D · `src/main.rs`: `mod ui_minimap`.
* D · `src/ui.rs`: `Action::ToggleFavourite`, `Action::UsedAsset`, `UiState::{favourites, recent_assets}`; `catalogue`, `logic_panel`, `catalogue_of` take `actions`; favourites / recently used groups and a right-click menu in the catalogue.
* D · `src/install.rs`: `Prefs::{favourites, recent_assets}`.
* D · `src/app.rs`: `OPENMAPPER_TOOL` also takes `roads` and `objects`.
* D · `src/ui.rs`: `Action::MapPicture`, `Action::SetMapCamera`; `map_properties` takes `st` and `actions` and shows the map's picture.
* D · `src/app.rs`: `read_frame` (split from `save_shot`), `save_map_picture`, `[mapcam]` from the camera.
* D · `src/ui.rs`: `Action::SetScale`, "Interface size" in `lang_menu`; `src/install.rs`: `Prefs::ui_scale`; `src/app.rs`: zoom applied at start and on change.
* B · `src/ui.rs`: `Action::Copy / Cut / Paste / Duplicate / SelectAll` and Edit menu entries; `check`, `section`, `prop_row`, `TEXT_DIM` made `pub(crate)` for `ui_edit.rs`; properties: several selected go to `ui_edit::group_properties`, object X / Y are numbers (`Editor::put_object`), a spline's ends via `ui_edit::spline_ends` / `apply_ends` (the old Gradient row went); status bar: `ui_edit::snap_controls`; Select hint and Controls rows.
* B · `src/app.rs`: Ctrl+C / X / V / D / A push the new actions and they are handled; Shift / Ctrl + left press on empty ground starts `ed.select_box`, cursor moves update it, release calls `ed.finish_box`; `ui_edit::paint_box` after `ui::editor`; undo / redo use `ed.select(None)`.
* B · `src/main.rs`: `mod clipboard`, `mod reshape`, `mod selection`, `mod ui_edit`.
* B · `src/render/mod.rs`: `FrameExtras::selected_more` (a `HashSet<Sel>`), tinted like `selected` and part of the scene key.
* B · `src/doc/mod.rs`: `Sel` derives `Hash`.
* B · `Editor::selected` stays the one shown in the properties; the rest of a selection is `Editor::also` (meaningless while `selected` is None). Use `ed.select(..)` / `ed.selection()` rather than writing `selected` when it matters.
* A · `src/main.rs`: `mod timetable`.
* A · `src/doc/mod.rs`: `Document::timetable` (read with the map in `load`, empty in `new_map`), counted in `is_dirty`.
* A · `src/doc/save.rs`: `crate::timetable::save(doc)` after `global.cfg` (writes only the changed TTData files).
* A · `src/main.rs`: `mod ui_timetable`.
* A · `src/ui.rs`: `Action::Timetable`; Map menu "Lines and timetable…" (key `L`).
* A · `src/app.rs`: `App::tt` (the Lines window): drawn after `ui::editor`, its route laid into the frame's extras after `Editor::extras`, `before_save` in `App::save`, `Action::Timetable` and key `L` toggle it; `OPENMAPPER_TIMETABLE=<line>[:tours]` opens it on a line, and with `OPENMAPPER_DEMO` adds stops and a line to the demo map (checks without a person at the screen); `Action::Force` also drops the timetable's changes.
* C · `src/main.rs`: `mod check;` and `openmapper --check path/to/global.cfg` (checks a map without a window).
* C · `src/ui.rs`: `UiState::check` (the map check window), Map > Check map…, the window drawn at the end of `editor()`.
* C · `src/app.rs`: `OPENMAPPER_PROFILE` timings round the editor, the interface, acquiring the frame and submitting it; the camera turning by itself with `OPENMAPPER_PROFILE=orbit`; screenshot and profiling runs go on when the window gets no redraws; `OPENMAPPER_CHECK=1` opens the map check.
* C · `src/gpu.rs`: no vsync while profiling.
* C · `src/ui.rs`: `Action::RestoreBackup(PathBuf)`, File > Earlier versions… (`UiState::versions_open`, the window is `doc::safe::versions_window`).
* C · `src/app.rs`: autosave (`App::autosave`, ticked after the frame's actions, not while the left button is down or an edit is open), the offer at start (`App::recover`, `doc::autosave::offer`), the autosave cleared on save, close, opening or making another map and a normal quit; `Action::RestoreBackup` handled (guarded like Open); the save error says the changes are still there.
* C · `src/timetable/tests.rs`: `one_map()` takes the crate-wide `doc::one_map()` lock (the grid is global: Spandau's tests and the round trips of other maps must not run at once).
* A · `src/editor.rs`: `Editor::{pick_mode, picked}`: with pick mode on, a left click with the Select tool only notes what is under the cursor and where (the Lines window picks stops so: nothing is selected or dragged).
* A · `src/history.rs`: `Step::{timetable, at}`, `Snapshot` and `History::push_snapshot` (a window's edit given as the state before it; edits of one kind within 1.5 s are one step); `apply` swaps the timetable too (`Timetable::restored` has every file written again).
* A · `src/main.rs`: `mod traffic`, `mod ui_traffic`.
* A · `src/doc/mod.rs`: `Document::traffic` (`ailists.cfg`, unscheduled traffic, parked cars, people; read in `load`, from the template in `new_map`), counted in `is_dirty`.
* A · `src/doc/save.rs`: `crate::traffic::save(doc)` after the timetable's (only the changed files).
* A · `src/selection.rs` (tests' `flat_map`): `traffic: Default::default()`.
* A · `src/history.rs`: `Step::traffic`, `Snapshot::traffic`.
* A · `src/ui.rs`: `Action::Traffic`; Map menu "Traffic and AI…" (key `T`).
* A · `src/app.rs`: `App::traffic` (the Traffic and AI window) drawn after the Lines window, `Action::Traffic` and key `T`; `OPENMAPPER_TRAFFIC=day|groups|lists` opens it on a tab; `Action::Force` drops the traffic's changes too.
* B (round 2) · `src/app.rs`: `U` toggles the ruler; the Select tool's empty-ground press goes to the editor while the ruler is on; `ed.paste(&self.lib)`.
* B (round 2) · `src/ui.rs`: the properties panel is in a `ScrollArea`; an object hanging on another (`[attachObj]`) is shown through `selection::any_object` / `any_object_mut` (no X / Y, a "Hangs on" row); a road's properties end with `ui_edit::rows_section`; Controls row for `U`.
* B (round 2) · `src/main.rs`: `mod rows`.
* A · `src/main.rs`: `mod sample`, `mod ui_guide`, `mod ui_newmap`.
* A · `src/ui.rs`: the New map dialog moved to `src/ui_newmap.rs` (`UiState::new_map` is a `NewMapForm`; `Action::NewMap` carries `options`), `Action::Guide` and Help menu "Getting started".
* A · `src/install.rs`: `Prefs::guide_off`.
* A · `src/app.rs`: `new_map` takes the options (base ground, left-hand traffic, sample → `App::sample`), the New map dialog drawn from here (both screens), `App::guide` window and its "Show me" requests, `Action::Guide`, Play ticks the guide's test drive; `OPENMAPPER_DEMO=SAMPLE:<name>` (with `OPENMAPPER_LHT` keeping left) makes a sample map and saves it, `OPENMAPPER_NEWMAP` opens the dialog, `OPENMAPPER_GUIDE` the guide.
* A · `src/ui_timetable.rs`: `add_depot` is `pub(crate)` (the sample uses it).
* A · `src/doc/save.rs`: the timetable's and the traffic's files are planned as `safe::Op`s (`timetable::plan`, `traffic::plan`) and committed with the map's own, all or nothing (`saved` marks them done afterwards); the separate `extra` backup list for them is gone (their ops are backed up like the rest).
* B (round 3) · `src/ui.rs`: `Action::BuildRoad`; the Roads panel calls `ui_edit::road_options` (keep height).
* B (round 3) · `src/app.rs`: Enter pushes `Action::BuildRoad` (handled with `ed.confirm_road`); Shift sets `ed.straight_lock` and `ed.replan`; `ui_edit::road_entry` drawn after `ui::editor`.
* B (round 3) · `src/main.rs`: `mod draw`.
* B (round 3) · `src/main.rs`: `mod roundabout`. Roads panel options (roundabout, keep height) come from `ui_edit::road_options`.
* A · `src/main.rs`: `mod export`; `--export path/to/global.cfg out.zip` and `--import file` run without a window.
* A · `src/ui.rs`: `Action::Export`, File menu "Export map for sharing…".
* A · `src/app.rs`: `App::export` (saves, asks where, packs; toasts what went in and what is missing).
* A · `src/check.rs` (C): `missing_textures` is `pub(crate)` (the export test checks the imported map with it).
* A · `src/doc/mod.rs`, `src/doc/save.rs` (C): `Document::descriptions` (`global_<LANG>.dsc`, new `src/descriptions.rs`), read with the map, counted in `is_dirty`, planned with the save's ops; `descriptions: Default::default()` in the tests' and autosave's `Document` literals (`src/doc/tests.rs`, `src/doc/autosave.rs`, `src/check.rs`, `src/selection.rs`).
* A · `src/ui.rs`: `map_properties` shows `descriptions::editor` (a language picker above the description) in place of the plain description field.
* A · `src/history.rs`: `Snapshot::descriptions`.
* A · `src/main.rs`: `mod descriptions`.
* A · (no new shared hooks for Holidays.txt: it is part of the timetable, `Timetable::calendar`.)
* B (round 4) · `src/junction.rs`: `Options` (lights, greens, crossings; kept as a comment line in the junction's `.sco`), `make_with` (light program, `[use_traffic_light]` on the first piece of each way in, signal heads and poles with `[varparent]`, zebra stripes and footways), `children`, `remove_unused`; `make` calls `make_with` with no options.
* B (round 4) · `src/ui.rs`: an object's properties end with `ui_edit::junction_section` (for openMapper's junctions).
* B (round 4) · `src/app.rs`: after a successful save, `ed.clean_junction_files()` deletes this map's junction files no tile uses that were made before this session.

* C · `src/install.rs`: `Prefs::save` writes through `doc::safe::write_atomic` (a warning in the log when it fails).
* C · `src/gpu.rs`: `SAMPLES` went; `Gpu::samples` and `Gpu::set_samples` (MSAA chosen by the graphics quality; 1 draws the view straight into the frame).
* C · `src/install.rs`: `Prefs::graphics` ("low" / "medium" / "high", empty: high).
* C · `src/ui.rs`: `Action::SetGraphics(render::quality::Quality)`, View > Graphics quality.
* C · `src/app.rs`: the renderer made with the quality from the settings (`Renderer::new(&gpu, quality)`, `OPENMAPPER_GRAPHICS` overrides for checks); `Action::SetGraphics` makes the renderer again (thumbnails freed); `OPENMAPPER_PROFILE=gpu` times the view on a picture of its own (`Renderer::time_gpu`).
* Geodata · `Cargo.toml`: `ureq` 3 (rustls, no other features) for imagery, heights and place search; `image` also reads TIFF, BMP and WebP (own pictures).
* Geodata · `src/main.rs`: `mod geo`, `mod ui_geo`.
* Geodata · `src/install.rs`: `Prefs::geo` (`geo::sources::GeoPrefs`: the user's own tile servers and keys).
* Geodata · `src/render/mod.rs`: `pub mod imagery`, `Renderer::imagery` (per-tile textures with mipmaps); the tile bind group has the tile's imagery at binding 5 and its sampler at 6 (`tile_bind`, made again when the tile's imagery changes: `GpuTile::{views, imagery_rev}`), its opacity is `TileUniform::flags[1]`. A new renderer (graphics quality) starts empty: `Imagery::epoch` changes and `ui_geo` uploads again from disk.
* Geodata · `src/render/terrain.wgsl`: bindings 5 and 6 of group 1, the imagery mixed into the ground's colour by its alpha times `tile.flags.y`.
* Geodata · `src/ui.rs`: `Action::Geo`, Map menu "Satellite and terrain…" (key `I`), `UiState::geo` (`ui_geo::Bridge`), View menu `ui_geo::view_menu`, the Tiles panel ends with `ui_geo::tiles_panel`; the Tiles hint mentions clicking a tile.
* Geodata · `src/app.rs`: `App::geo` (`ui_geo::GeoUi`): `fill_bridge` before the interface, `window` after the Traffic window (its `look` becomes `Action::LookAt`), `take_bridge` after it, `feed` before `Renderer::sync`; key `I`, `Action::Geo`; a picture dropped with the Tiles tool on goes to `geo.drop_picture` first; the screenshot waits while `geo.busy()`; `OPENMAPPER_GEO=lat,lon[,load][,hide][,picture=<file>[,turn=<deg>]]`.
* Geodata · `src/editor.rs`: `Editor::picked_tile` (the tile clicked or added last with the Tiles tool).
* Geodata · `src/export.rs`: `plan` calls `geo::export_filter` (the map's `openMapper/imagery` stays out of shared zips unless the map's geo.json says `export_imagery`).
* Geodata (round 2) · `src/ui_minimap.rs`: `Minimap::imagery` (each tile's small picture, drawn over its square); `src/app.rs` fills it from `geo.minimap_tiles()` before the interface. `src/ui.rs`: Controls row for `I`.
* Geodata (round 2) · Google-sourced pictures are never written (map folder, backups, exports): `ImageryMeta::kept` false, held in memory and made again from the tile cache (Google's max-age) when the map opens.
* Review · `src/ui.rs`: `Action::SaveThen(Box<Action>)` (the unsaved-changes prompt's Save: the action follows only if the save worked); `src/app.rs`: `Action::Force` passes the guard by (`App::unguarded`) instead of clearing the dirty flags, `Action::Undo / Redo` call `Editor::undo / redo` (they stop the tool first).
* Review · `src/doc/save.rs`: `crate::timetable::follow_stop_tiles(doc)` before the timetable's plan (a stop object moved into another tile).
* Review · `src/history.rs`: `History::object_files` (the junction cleanup after a save keeps what undo and redo can bring back).
* D · `src/main.rs`: `mod texts`, `mod parallel`, `mod ui_find`, `mod ui_groups`; `Cargo.toml`: `omsi-content` (the `.oft` fonts).
* D · `src/assets.rs`: `ObjectModel::{texts, labels, editor_parts}`, `Part::texts`; a text texture is asked for as a texture under a path `texts::path` makes, drawn by `texts::draw` in the loader; `load_object` is `pub(crate)`.
* D · `src/render/mod.rs`: `Renderer::model_with_labels` (a model copy per text shown), `FrameExtras::hide` (trees, objects, splines), `hidden_layer`, `push_object(.., editor)`, the tiles' water kept among the spline meshes under the IDCode `WATER`.
* D · `src/ui.rs`: `UiState::{find, groups, history_open}`, `ObjInfo::labels`; Edit menu "Edit history…", "Groups…", "Find in map…" (Ctrl+F); View > Show; the Tiles panel's water; a spline's length, radius and "Select the whole road".
* D · `src/editor.rs`: `Editor::{shape_request, hide}`, `set_spline_cant`, `parallel_copy`, `replace_everywhere`, `even_selected_chain`, `select_chain` (a double click on a road), `grow_map`, `set_water`, `turn_objects`.
* D · `src/doc/mod.rs`: `TileDoc::water` (read and written with the tile's `.water`); `doc::save` is `pub(crate)` (`tile_text` and `utf16` for groups).
* D · `src/app.rs`: `OPENMAPPER_SELECT=x,y[,spline]`, `OPENMAPPER_HIDE`, `OPENMAPPER_FIND`, `OPENMAPPER_GROUPS`, `OPENMAPPER_HISTORY`, `OPENMAPPER_DRIVE` (the Test drive window open), `OPENMAPPER_WIDTH=<points>` (the window's width); a screenshot run takes no input and does not take the focus; `Prefs::save` does nothing in one.
* D · `src/editor.rs`: `array` (copies in a row), `align_to_roads`, `replace_selected_objects`, `select_alike`; `camera::Camera::axes` is public (Alt + arrows nudge along the screen's nearest map axis).
* D · `src/app.rs`: `snapshot` (View > Save a picture of the view, F12, into `~/Pictures/openMapper`), `OPENMAPPER_SNAPSHOT`; `ui::map_stats` in the map's properties.
* D · `src/main.rs`: `mod ui_mapsettings`, `mod ui_calendar`; `src/ui.rs`: `Action::{MapSettings, Calendar}` (Map menu), `ObjInfo::{paths, attach_points}`; `src/app.rs`: `OPENMAPPER_SETTINGS[=calendar]`, `OPENMAPPER_LEVEL`.
* D · `src/traffic/mod.rs`: `Traffic::{calendar, timezone, registrations, has}` with `File::{Holidays, TimeZone, Registrations}` (written with the map, undone with the traffic).
* D · `src/editor.rs`: `set_rules`, `set_spline_extra` (skew, terrain align), `level_request` / `level_under_objects`, `attach_object`.
* D · `src/main.rs`: `mod signals`, `mod ui_signals`, `mod ui_chrono`, `mod chrono_edit`; `src/ui.rs`: `Action::{SignalRoutes, Chrono}` (Map menu); `src/app.rs`: signal pieces counted again before saving, `OPENMAPPER_SETTINGS=signals|chrono`.
* D · `src/traffic/mod.rs`: `Traffic::signal_routes` with `File::SignalRoutes`; `src/doc/mod.rs`: `Document::{chrono_root, show_chrono}`; `src/doc/save.rs`: `object`, `spline`, `header`, `copy_missing` are `pub(crate)` (chrono patches); `src/check.rs`: `Kind::SignalRouteBroken`.
* D · `src/app.rs`: `ChronoWindow::end_session` before saving and before closing or replacing the map; autosave waits while a scenario is edited on the map. `src/doc/safe.rs`: `own_path` (write targets inside a map's own folder; `resolve_path` may lead to another content root). `src/doc/mod.rs`: `show_chrono_without`.
* D · `src/main.rs`: `mod chrono_tt` (a scenario's timetable and AI lists sessions); `src/ui.rs`: `Action::{OpenLines, OpenTraffic}`; `src/timetable/mod.rs`: `renumber_tiles` is `pub(crate)`; `src/traffic/mod.rs`: the holidays are the timetable's only (`File::Holidays` went); `src/doc/mod.rs`: `one_map` gives back the plain grid when let go (tests).
* D · `src/main.rs`: `mod heightmap`; `src/ui.rs`: `UiState::heightmap` (its buttons at the end of the Terrain panel), `BG_FIELD` is `pub(crate)`; `src/editor.rs`: `Editor::flatten_fixed`; `src/app.rs`: `OPENMAPPER_TOOL` also takes `terrain` and `paint`, `OPENMAPPER_HEIGHTMAP=<picture>[,apply]`.
* D · `src/main.rs`: `mod autopaint`; `src/ui.rs`: `UiState::autopaint` (its rule at the end of the Paint panel); `src/app.rs`: `OPENMAPPER_AUTOPAINT=<layer>`.
* D · `src/editor.rs`: `reverse_selected`; `src/reshape.rs`: `reversed`; `src/ui.rs`: a spline's "Reverse direction" (`ui_edit::reverse_help`), also for several in `group_properties`.
* D · `src/render/mod.rs`: `pub mod daylight`, `Renderer::daytime` (the light of the view from it; `light_view` takes the sun, the shadow pass uses the globals' sun); `src/ui.rs`: `UiState::daytime`, View > Time of day; `src/app.rs`: the renderer's daytime set before `sync`, `OPENMAPPER_DAYTIME=<hour>`.
* D · `src/main.rs`: `mod onground`; `src/editor.rs`: `lay_selected_on_ground`; `src/ui.rs` / `src/ui_edit.rs`: "Lay on the ground" for a road and a selection (`ui_edit::lay_help`); `src/app.rs`: `OPENMAPPER_LAY=1`.
* D · `src/main.rs`: `mod osm`, `mod ui_osm`; `src/ui.rs`: `Action::Osm` (Map menu); `src/app.rs`: `App::osm` drawn after the Satellite window, its build request run after the interface (`Editor::build_osm` with the assets), `OPENMAPPER_OSM=<main type>[;<street type>]`, `OPENMAPPER_SAVE=1` (saved at the third frame); `src/editor.rs`: `build_road`'s body is `lay_road` (no step of its own), `build_osm`; `src/ui_geo.rs`: `GeoUi::georef` is `pub(crate)`; `src/geo/fetch.rs`: `overpass`.
* D · `src/main.rs`: `mod osm_lines`; `src/app.rs`: after the OSM roads and stops, the bus routes into the timetable (`osm_lines::add`, one snapshot step); `Editor::build_osm` takes the one-way type, `Editor::place_osm_stops`.
* D · `src/main.rs`: `mod osm_green`; `src/editor.rs`: `plant_osm`; `src/app.rs`: the OSM build plants the trees last.
* D · `src/assets.rs`: `Material::glow` (`[matl_nightmap]` / `[matl_lightmap]`); `src/render/mod.rs`: the material's binding 5 is its glow texture, `MatUniform::mode[3]` its kind, `Globals::shadow[2]` how dark it is (`Light::night`).
* D · `src/ui.rs`: `Action::SetSeason`, `UiState::season`, View > Season; `src/app.rs`: `set_season` (omsi-texture's season folder, the assets and models read again), `OPENMAPPER_SEASON`; `src/render/mod.rs`: the ground's textures looked for in the season's folders first.
* D · `src/main.rs`: `mod osm_buildings`; `src/editor.rs`: `place_osm_buildings`; `src/app.rs`: the OSM build puts the buildings before the trees; `OPENMAPPER_OSM`'s seventh field a building folder (the catalogue is read first).
* D · `src/history.rs`: `mark` and `merge_since` (the OSM build is one step). `src/main.rs`: `mod osm_water`; `src/osm_green.rs`: `rings` is `pub(crate)`; `src/app.rs`: the OSM build digs the water after the roads (they keep their heights over it).
* D · `src/editor.rs`: `Editor::{plant_area, plant_outline}`, `finish_plant_outline` (Enter via `confirm_road`, Esc and right-click take it back); `plant_osm` calls `plant_spots`; `src/app.rs`: `ed_has_pending` counts the outline, `OPENMAPPER_PLANT=<plant>;x,y;…[;open]`.
* D · `src/main.rs`: `mod ui_profile`; `src/ui_edit.rs`: `group_properties` takes `actions` (the height profile after Even slope); `src/ui.rs`: a single road's properties end with `ui_profile::profile_section`; `src/app.rs`: `OPENMAPPER_SELECT=x,y,chain`.
* D · `src/main.rs`: `mod ui_views`; `src/ui.rs`: `UiState::views`, View > Saved views…; `src/app.rs`: the window drawn after the Satellite window (`follow` the map's folder), `Ctrl(+Shift)+1…9`, `OPENMAPPER_VIEWS=<n>`.
* D · `src/main.rs`: `mod ui_palette`; `src/ui.rs`: `UiState::palette`, Edit > Search for a command…, drawn after the edit history, `SELECT` is `pub(crate)`; `src/app.rs`: `Ctrl+K` / `Ctrl+Shift+P`, `OPENMAPPER_PALETTE=<text>`.
* D · `src/install.rs`: `Prefs::last_views` (`remember_view`, `last_view`); `src/app.rs`: `remember_view` before another map opens, on Close and Quit; `after_open` goes there (not in screenshot runs).
* D · `src/main.rs`: `mod merge`, `mod ui_merge`; `src/doc/mod.rs`: `load_tile` is `pub(crate)`, `tests` is `pub(crate)` (`scratch`, `sample_map` for other modules' tests); `src/ui.rs`: `Action::MergeTiles`, `UiState::merge` (drawn after Groups), Map > Tiles from another map…; `src/app.rs`: `Action::MergeTiles` lists the maps, `OPENMAPPER_MERGE=<global.cfg>[,add]`.
* D · `src/ui.rs`: `Action::{RevealMap, CopyMap, BinMap}` from a right-click on the welcome list; `src/app.rs`: `copy_map`, `bin_map`.
* D · `src/doc/mod.rs`: `TileDoc::new_flat` marks the tile `[variable_terrainlightmap]`; `src/editor.rs`: `set_tile_lamplight`; `src/ui.rs`: the picked tile's section (`water_panel`) has the switch.
* D · `src/library.rs`: `Asset::lamp` (`[maplight]`); `src/check.rs`: `Kind::LampsDark`, `Fix::LampLight`.
* D · `src/main.rs`: `mod roadpaint`; `src/autopaint.rs`: `paint_where` (a layer painted by any weight; `apply` uses it), `band` is `pub(crate)`; `src/editor.rs`: `Editor::paint_along`, `paint_along_selected`; `src/ui_edit.rs`: `paint_along_section` (a road's properties and a selection's); `src/app.rs`: `OPENMAPPER_PAINT_ALONG=<layer>,<width>[,under]`.
* D · `src/editor.rs`: `Editor::{stop_both_sides, stop_twin}` (planned with the stop, placed with it, a ghost of its own); `src/ui.rs`: the Bus stops mode's switch; `src/app.rs`: `OPENMAPPER_STOP=x,y[,one]`.
* D · `src/main.rs`: `mod plan`, `--plan global.cfg out.svg`; `src/ui.rs`: `Action::ExportPlan`, File > Export a plan of the map…; `src/app.rs`: it saves where asked and shows the file.
* D · `src/main.rs`: `mod trace`, `mod ui_trace`; `src/editor.rs`: `Editor::{trace_lines, look_request}` (drawn in `extras`; a window's "go there"), `build_osm` waits for the map's own road models too (junctions need their kind); `src/ui.rs`: `Action::Trace`, Map > GPS tracks to trace…; `src/app.rs`: `App::trace` drawn after the OSM window, its build taken after the interface, `.gpx` / `.kml` / `.geojson` dropped go to it, `OPENMAPPER_TRACE=<file>[,build]`.
* D · `src/editor.rs`: `wire_along`; `src/ui_edit.rs`: a track's Overhead wire under Parallel copy; `src/app.rs`: `OPENMAPPER_WIRE=<spline>`.
* D · `src/main.rs`: `mod gamecheck`; `src/ui.rs`: `Action::GameCheck`, Map > Check in openOMSI…; `src/app.rs`: `game_check` (saves, runs the game offscreen with `--schedule --view free --cam` from the view, `--all` on small maps, its `OMSI_CHECK_*` flags), the window after the trace window, `OPENMAPPER_GAMECHECK=1`.
* B · `src/editor.rs`: `Editor::link_pick` (the next click in the view picks an object's `[varparent]`), `set_var_parent`, `set_light_index`; links drawn for one selected object. `src/app.rs`: Escape calls linking off first; `UiState::parent_lights` filled. `src/ui.rs`: `ObjInfo::{lights, light_head}`, `ui_edit::link_section` in an object's properties.
* B · `src/ui.rs`: `UiState::outliner` (new `src/ui_outliner.rs`), the Select panel's Tools / Objects tabs; `src/app.rs`: `OPENMAPPER_OUTLINER=[search]`; `src/main.rs`: `mod ui_outliner`.
* B · `src/editor.rs`: `Editor::groups` (new `src/mapgroups.rs`), a click grows to its group, `right_click` returns whether the view's menu opens; `src/ui.rs`: `Action::{PasteAt, SavedGroups}`, `UiState::view_menu` (new `src/ui_menu.rs`); `src/app.rs`: Ctrl+G / Ctrl+Shift+G, `OPENMAPPER_MENU=x,y`.
* B · `src/editor.rs`: `Editor::{road_at, cut_road_at, cut_spline, join_with_next}`; `src/doc/mod.rs`: `Document::join_splines`; `switches::spline_at` made `pub`; `src/ui_menu.rs`: "Cut the road here", "Join with the next piece"; `src/ui.rs`: a spline's Cut (the knife) / Join with the next piece, the knife's hint and Controls row; `Editor::{cutting, toggle_cutting, cut_preview}` (K, Esc and right-click put it away), `ui_edit::{paint_cut, knife_help}`, the status bar's Knife switch, a command in the search; `src/app.rs`: `OPENMAPPER_MENU=x,y[,gx,gy]` (the ground clicked), `OPENMAPPER_CUT=x,y`, `OPENMAPPER_JOIN_NEXT=<n>` (after `OPENMAPPER_SELECT`), `OPENMAPPER_KNIFE=x,y` (the knife on, the cursor held there: `App::pinned_cursor`).
* D · `src/main.rs`: `mod hof`, `mod ui_hof`; `src/traffic/mod.rs`: `Traffic::depots` (the map folder's `.hof` files, read with the map, written with `File::Depots`, a file the map no longer has taken away; undone with the traffic), `share_depots` (a copy in the content's `HOFs`, never over a file openMapper did not write), called from `doc::save::save`; `src/ui_traffic.rs`: `Tab::Depots`; `src/ui.rs`: `Action::Depots` (Map menu "Depot files"); `src/app.rs`: `OPENMAPPER_TRAFFIC=depots[,make][,stops|routes]`; `src/check.rs`: `Kind::DepotFile`, `Fix::{MakeDepot, FillDepot, OwnDepot}`. Map settings' Depot lists the AI's depot groups (`[standarddepot]` names one), not `.hof` files.
* B · new crate `crates/mesh` (`openmapper-mesh`: polygon meshes, primitives, CSG, modifiers, edit-mode operations, UV unwrap and pack, baking, export to OBJ / glTF / GLB / FBX / DAE / USDA / STL / PLY / 3MF / OFF / X / O3D), a path dependency; glam's `serde` feature on.
* B · new `src/gizmo.rs` (screen-space Move / Rotate / Scale handles, X Y Z, plane squares, rings), `src/scaled.rs` (scaled copies of a type, `name~sX_Y_Z.sco`), `src/ui_xform.rs` (Location / Rotation / Scale rows); `src/editor.rs`: `Drag::{Gizmo, MeshGizmo}`, `Editor::{gizmo, gizmo_view, gizmo_under_cursor, set_gizmo_view, apply_scales, drag_readout, stop_drag, scale_preview, scale_requests, mesh}`; `src/render/mod.rs`: `FrameExtras::{resized, solid, overlay_depth}`, `Renderer::forget_model`; `src/assets.rs`: `Assets::forget_object`; `src/app.rs`: a press on a handle goes to the editor, `OPENMAPPER_XFORM=move|rotate|scale`; `src/controls.rs`: `Cmd::{XformMove, XformRotate, XformScale, EditMesh, JoinObjects}` (V, C, X, Tab, Ctrl+J).
* B · new `src/editable.rs` (openMapper objects: `<name>.openmapper.json` beside the `.sco` and `.x` written from it, any object made one), `src/meshops.rs` (make editable, make single, separate parts, join, booleans), `src/meshedit.rs` (the edit mode's session), `src/signal_maker.rs` (signal head, street lamp), `src/ui_mesh.rs` (shapes, the mesh panels, the edit mode's panel), `src/ui_uv.rs` (UV editor), `src/app_mesh.rs` (a child module of `app`: `App::{mesh_frame, mesh_key, mesh_action, toggle_edit_mode, write_mesh, forget_object}`); `src/ui.rs`: `Action::{CreatePrimitive(usize), Mesh(MeshAction)}`, `UiState::{bake, keep_cutters, uv_open, uv, signal_lamps, scale_uniform}`; `src/app.rs`: `OPENMAPPER_SHAPE=<n>|signal|lamp,x,y`, `OPENMAPPER_EDIT=vertex|edge|face[,all|top][,extrude|inset|bevel|subdivide|modifier|leave]`.
* B · `src/import.rs`: a model becomes an openMapper object in `Sceneryobjects\openMapper\Imported` (a part for each node, standing on the ground); a folder with only `texture` beside its models is not a mod; `.sco` folders hand back their objects; models found anywhere in a folder or zip; `materials_from` (an `.mtl`, a model's materials, a set of PBR pictures).
* B · `src/editor.rs` `build_osm`: plates there before joined, junctions where streets end at one node (`junctions_at_nodes`), switches where tracks branch with Auto-Switch; `src/ui_newmap.rs`: description, ground height, busy streets, years, money, tickets, seasons, real rail, start from a real place (the sample left out; `OPENMAPPER_DEMO=SAMPLE:` still makes it).
* Languages · `src/i18n.rs`: `Lang` has the twenty-eight languages of openOMSI plus Serbian Latin, `LANGUAGES` lists them (code, name in itself, aliases), `by_name` / `from_code` find one by any spelling (and the system's locale: `LANG`, macOS `AppleLanguages`, Windows `GetUserDefaultLocaleName`); every language but English and Russian is a table found by the English text, `locales/<code>.tsv`, built the first time it is asked for. `tr()` call sites do not change.
* Languages · `src/sysfont.rs`, `src/icons.rs`: Chinese, Japanese, Korean, Thai and Hindi are drawn with the system's own font, put behind the interface font in `icons::fonts()` (made again by `Action::SetLang`); without one `i18n::set_drawable(false)` keeps the texts in English.
* Languages · `src/ui.rs`: View > Language lists `i18n::LANGUAGES`; `src/ui_palette.rs`: one `lang_cmd!` command per language.
* Languages · `tools/i18n/check.py`, `.github/workflows/ci.yml` (job `translations`): no `tr()` text or `cmd!` command without a translation in every `locales/*.tsv`.
* Objects · `src/render/mod.rs`: object meshes (`GpuMesh::one_sided`) are drawn with `mesh_opaque_cull` / `mesh_blend_cull` (back faces culled, front face clockwise, as openOMSI's `one_sided_primitive`); roads, ground, overlays and thumbnails stay two-sided.
* Panels · `src/ui.rs`: `long_label` (breaks anywhere) and `short` (cut with an ellipsis) keep a long name or path inside a panel.
* Junctions · `src/junction.rs`: `mesh_x` writes every triangle twice, the second facing the other way.
* Speed · `src/library.rs`: `Library::find` lower-cases on the stack and hashes with `Fx` (`paint_names` asked it for every object every frame); `src/ui.rs`: the map picture's file is looked at once a second.
* Stops · `src/stops.rs` (new): a stop's settings in the strings of its object (name, people waiting at most and at least, getting off, length, side, kind as the seventh string), pairs and unique names; `src/ui_stops.rs` (new): the stop's panel; `src/ui.rs`: the panel under a stop's name, `Editor::stop_kind` and the kind picker of the Game logic tool; `src/editor.rs`: new stops take their strings from `StopSettings::new_strings` and a name no far stop has; `src/check.rs`: `Kind::StopName`, `Fix::UniqueStopName`; `src/ui_timetable.rs`: "How the stops are served" (`[profile_otherstopping]`, kept with its station when stops move).
* Vehicles · `src/timetable/caruse.rs` (new): `car_use/*.ocu` as part of the timetable (`Timetable::car_use`, written with the TTData files, in undo / redo); `src/ui_timetable.rs`: the Vehicles tab of a line.
* Roads · `src/road_maker.rs`: `RoadSpec` (median, footways each side), `presets`, `Styles::pieces`; `src/ui_roads.rs`; `src/ui_edit.rs`: `end_junction_section`; `src/ui.rs`: `Action::JunctionAtEnd`; `src/editor.rs`: `Editor::junction_at_end`; `src/library.rs`: `Library::junction_pieces`; `src/assets.rs`: `Assets::object_wait`. Hook `OPENMAPPER_JUNCTION_END=start|end` puts the first fitting piece on the road `OPENMAPPER_SELECT=x,y,spline` selected.
* Scripts · `src/scripting.rs`, `src/ui_script.rs` (new): `Action::Scripts`, `Action::ScriptedLight`, Map menu "Scripts…", `App::scripts`; `src/export.rs`: the map's `plugins/map_<folder>` is packed; `OPENMAPPER_SCRIPTS=1` opens the window. New dependencies: `full_moon` (Lua 5.4 syntax) and `omsi-script` (openOMSI's compiler of `.osc`).
* Roads · `src/pocket.rs`, `src/cjunction.rs`, `src/ed_cjunction.rs`, `src/ui_cjunction.rs` (new): bus pockets (`Editor::pocket_click`, the width adapter's taper ratio `adapt_joints_by`), complex and custom junctions (`Editor::place_layout`, `apply_custom` called by `make_junctions` before it makes a plate); `src/editor.rs`: `Editor::{bus_pocket, pocket_type, pocket_len, custom_layouts, use_custom}`, `conform_disc` and `reach_along` made `pub(crate)`; `src/ui.rs`: `Action::{ComplexJunctions, PlaceComplex, SaveCustomJunction}`, Tools menu entry; `src/ui_edit.rs`: `pocket_options` in the Roads tool's panel; `src/app.rs`: `App::cj`, the actions, `app::mesh` made `pub(crate)` for `mesh_of`, `OPENMAPPER_CJ=<layout.json>` opens the complex junction editor on a drawing, `OPENMAPPER_MAKER=[pocket]` opens the road maker; `src/assets.rs`: `Assets::spline_wait`; `src/road_maker.rs`: `RoadSpec::pocket`; `src/ui_roads.rs`: top view / side view of the road made.
* Hooks · `OPENMAPPER_POCKET=<dx>,<dy>[,<width>]` (with `OPENMAPPER_DEMO=SAMPLE:<name>`) lays a bus pocket on the sample ring, `OPENMAPPER_CJ_LANES=1` draws the lanes in the complex junction editor, `OPENMAPPER_NOGUIDE=1` closes Getting started.
