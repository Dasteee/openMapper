# OSM importer overhaul: technical plan and agent handoff

Prepared: 2026-11-08. Repository inspected: openMapper 0.1.23.

## Purpose and status

This document began as an investigation and implementation plan. The source analysis below records the investigation baseline; see the implementation update for subsequent work. The original request was strictly research and planning; the user subsequently authorized saving this document. No importer fixes were implemented, application runs or tests performed, or commits made during that investigation.

Future agents should verify the current source before acting: line numbers and architecture may change. The user subsequently authorized the Phase 0/1 acquisition implementation on 2026-11-08. Do not interpret proposed capabilities or acceptance targets as existing behavior or measured results.

**Recommended order: repair and instrument acquisition; preserve OSM topology and tags; automate road selection and driveable junctions; automate buildings; then harden city-scale imports.** Reuse existing generation and format-handling code rather than starting over.

Evidence labels used below:

- **Confirmed:** observed in the inspected repository.
- **Hypothesis:** plausible explanation requiring reproduction or measurement.
- **Proposed:** future design or acceptance target.

## 1. User goals

- Fix imports that remain at “Downloading…” for roughly 6–7 minutes and then report that OSM is busy, including a reported 5×3-tile selection.
- Reliably import neighborhoods and larger city areas without repeated unnecessary downloads.
- Automatically select appropriate installed OMSI road assets from actual road characteristics rather than one manual mapping per highway category.
- Preserve traffic topology and produce roads usable by OMSI AI.
- Automatically select and place buildings from installed scenery, supplemented by procedural geometry where useful.
- Retain automatic defaults, manual overrides, meaningful diagnostics, cancellation, and inspectable results.
- Support OMSI 2 and openOMSI, and evaluate Kragujevac, Serbia as a representative real-world case.

## 2. Current-state analysis

Paths below are relative to the repository root. Function names are the durable navigation anchors.

| Stage | Existing implementation | Implication |
|---|---|---|
| Area | `src/ui_osm.rs::bbox` transforms tile corners and takes their enclosing geographic rectangle. | Disconnected/sparse tiles can request much more territory than their combined area. |
| Orchestration | `OsmUi::window` launches a thread and fetches routes, vegetation, buildings, water, then roads/stops. | Roads wait behind four supplementary datasets. |
| Acquisition | `src/geo/fetch.rs::overpass` serializes requests, uses disk cache, then tries two endpoints. | Caching and fallback already exist. |
| Road interpretation | `src/osm.rs::{query, parse, roads, join, order, segments}` select classes, clip, join, order, simplify, and fit straights/arcs. | Useful geometry baseline; most OSM semantics are lost. |
| Road construction | `src/editor.rs::Editor::build_osm` uses normal road construction, terrain, anchoring, junctions, and optional switches. | Reuse behind a stronger planning layer. |
| Junctions | `src/junction.rs`, `src/cjunction.rs`, `src/roundabout.rs`, editor integration. | Generated surfaces and traffic/pedestrian paths already exist. |
| Buildings | `src/osm_buildings.rs`, `Editor::place_osm_buildings`. | Size/height matching already exists, but requires chosen folders and has weak occupancy checks. |
| Other scenery | `src/osm_green.rs`, `src/osm_water.rs`, `src/osm_lines.rs`, stop placement. | Preserve vegetation, water, stops, routes, and timetable generation as optional stages. |
| Application | `src/app.rs` consumes `self.osm.build`. | Download is background work; substantial generation runs through the application update path. |

### Existing reusable infrastructure

- `src/library.rs::Library::start_scan`, `scan`, `describe`: background recursive `.sli`/`.sco` discovery in content and OMSI roots. Content overrides installation assets with the same relative path.
- Spline catalogue descriptors already contain visible-profile half-width and street, pedestrian, and rail path counts.
- `src/assets.rs`: existing openOMSI-backed readers expose model bounds, paths, profiles, materials, and dependencies.
- `src/road_maker.rs::RoadSpec`: forward/backward lanes, lane width, median, asymmetric footways, kerbs, textures, and `.sli` generation.
- `src/geo/job.rs`: progress messages, cancellation state, and worker-panic reporting patterns.
- `src/history.rs`, `src/doc/transaction.rs`, document save machinery: undo and mutation integration.
- `src/check.rs`, `src/gamecheck.rs`: static and game-validation integration.
- `src/export.rs`: dependency packaging and installation/content distinction.
- `src/models.rs`, `crates/mesh`: model import, geometry, and asset generation foundations.
- `Cargo.toml` already pins openOMSI format/geometry crates. Avoid replacing them with a second OMSI parser stack.

## 3. Download investigation

### Confirmed transport behavior

Endpoints, in order:

1. `https://overpass-api.de/api/interpreter`
2. `https://maps.mail.ru/osm/tools/overpass/api/interpreter`

`overpass` uses HTTP GET with a percent-encoded `data` query parameter and application User-Agent. Existing thematic queries declare `[out:json][timeout:90]`. The dedicated HTTP agent has a 150-second global request timeout; response reading is limited to 64 MiB.

There is one attempt per endpoint. This function does **not** use the generic downloader's separate three-attempt retry loop.

Five sequential datasets, each trying two endpoints, give a nominal request budget of:

```text
5 × 2 × 150 seconds = 1,500 seconds = 25 minutes
```

This excludes waiting for the global mutex and local processing. It is a code-derived budget, not measured runtime or a guaranteed upper bound. The reported 6–7-minute wait is consistent with this orchestration without requiring a large road query.

HTTP 429, 502, 503, and 504 become the same “servers are busy” message. Bodies are discarded and only the final endpoint's failure survives. Available Overpass slots do not guarantee admission under time/memory resource limits.

### Confirmed reliability gaps

- Global Overpass mutex acquired before cache access: even cache hits wait behind network work.
- `src/osm_map.rs` uses the same acquisition function and can contend with imports.
- No cancellation or intermediate progress for OSM downloads.
- All `try_recv` errors treated alike; disconnected worker can leave the UI waiting indefinitely.
- HTTP 200 responses with Overpass `remark` are excluded from caching but still returned to parsers, potentially accepting partial data.
- Valid empty datasets are not cached.
- Cache writes are non-atomic; a damaged fresh entry can be returned.
- Supplementary errors retain category names, not detailed causes.
- No overall import deadline.
- Road derivation keyed by tile count; other derived data keyed by tile count/download counter. Same-count tile or georeference changes can leave stale results.
- Building placement synchronously waits up to 60 seconds for models; road preparation also waits synchronously.

The current cache stores successful nonempty responses for seven days under a hash of exact query text. It does not reuse overlapping spatial coverage or store a resumable import manifest.

### Ranked explanations

| Explanation | Evidence | Verification |
|---|---|---|
| Supplementary requests and fallback timeouts consume most elapsed time. | Confirmed mechanism; likely contributor. | Per-dataset/per-endpoint timing. |
| Final road request receives 429 or gateway/resource failure. | Likely if the reported message matches this code. | Exact status and bounded body excerpt. |
| Secondary endpoint slow/unreachable from user's network. | Unverified. | One bounded identical small request against each endpoint. |
| Relations expand beyond bbox and increase cost. | Plausible. | Compare response bytes and remarks per dataset. |
| Background OSM map holds the mutex. | Confirmed possibility; occurrence unknown. | Queue timing and request owner. |
| Georeference or sparse tiles cause oversized bbox. | Plausible. | Log geographic bounds, physical size, tiles, georeference stamp. |
| Malformed query or GET URL size. | Lower priority for these short queries. | Replay exact query and compare POST; do not assume POST fixes it. |

### Required reproduction

Use the user's exact 5×3 selection, not an approximate replacement:

1. Record revision, georeference, tile size/keys, bbox, cache state, and enabled layers.
2. Replay each generated query independently with bounded time and restrained retries.
3. Record queue time, endpoint, attempt, HTTP status, elapsed time, bytes, content type, remark, and parse outcome.
4. Compare roads-only with the current five-stage sequence.
5. Compare cold and warm cache.
6. Compare background OSM map enabled/disabled.
7. Separate transport timeout, server rejection, incomplete response, and parse failure.

**The particular user's live failure was not reproduced during this investigation.**

## 4. Acquisition design

Start with a small OSM job coordinator using `geo/job.rs` patterns; do not require a large async rewrite upfront.

```text
Queued → Cache lookup → Requesting → Reading → Validating → Parsing → Ready
                                 ↘ Failed / Cancelled / Incomplete
```

Immediate requirements:

- Roads/stops first; only selected supplementary layers requested.
- Publish completed datasets independently.
- Distinguish empty, failed, and incomplete results.
- Preserve successes when another layer fails.
- Handle worker disconnection explicitly.
- Cache lookup before network queue; validate cached bodies.
- Atomic cache writes, including valid empty results.
- Retry failed layers, use cached data, and load local input actions.
- Structured diagnostics with endpoint, dataset, query identifier, status, timing, and bounded server explanation.

Use separate connection/read limits and an overall attempt/job budget. Tune server timeout and transport overhead from measurements instead of increasing the current timeout. POST is appropriate for extensibility and URL-size avoidance, but is not itself the root-cause repair.

Retry policy:

- 429: honor available retry guidance and back off.
- 502/503: bounded retry/fallback.
- 504: inspect body and context; distinguish resource rejection from gateway failure, then reduce demands or split where appropriate.
- Query errors: fail actionably without repeated unchanged retries.
- HTTP 200 with runtime remark: incomplete/error, not silent success.
- DNS/TLS/connect errors: specific diagnostics and bounded fallback.

Cancellation must cover queue waits, backoff, body reading, parsing, and generation. A flag checked only between 150-second blocking requests is insufficient. Replace transport behind an interface if needed to meet the cancellation target.

### Larger areas and local files

Use adaptive geographic chunks with context margins, merging by element type and OSM ID. Assemble topology and relation geometry before clipping.

- Keep crossing ways intact; deduplicate elements and building parts.
- Avoid huge enclosing rectangles for sparse selections.
- Detect conflicting versions/timestamps where available.
- Bound subdivision, attempts, bytes, and memory.
- Preserve completed chunks in a resumable manifest.
- Do not use public endpoints as an unlimited bulk-scraping backend.

Local PBF is the preferred repeatable path for larger areas. Support saved responses and OSM XML/PBF through a common normalized dataset. Geofabrik provides a Serbia extract suitable for Kragujevac evaluation.

Support configurable endpoints, including evaluating `https://overpass.private.coffee/api/interpreter`. Endpoint availability from the user's machine remains unverified. Sustained usage should support self-hosted or contracted infrastructure rather than assume a public-service SLA.

## 5. Architecture and invariants

These are logical boundaries; a small module tree is sufficient initially.

| Component | Responsibility | Integration |
|---|---|---|
| `OsmSource` | Overpass, saved responses, XML/PBF adapters. | Extract from `geo/fetch.rs` with compatibility adapter for existing callers. |
| `ImportJob` | Scheduling, progress, cancellation, retries, layer results. | `ui_osm.rs`, `app.rs`, `geo/job.rs` patterns. |
| `OsmDataset` | Typed nodes/ways/relations, tags, completeness, provenance. | Incrementally replace lossy parsing. |
| `AssetIndex` | Persistent road/building descriptors and dependency fingerprints. | Extend library; reuse CPU readers. |
| `RoadGraph` | Directed connectivity, junctions, layers, restrictions, semantic segments. | Reuse suitable `osm.rs` geometry. |
| `ImportPlan` | Proposed assets, transforms, geometry, confidence, warnings. | Immutable preview before mutation. |
| Apply layer | Check document revision, apply changes, undo/cancel safely. | Editor/history/document machinery. |
| Provenance | Source IDs, settings, generated IDs, overrides. | Project metadata/save/export. |

Invariants:

- Downloads never mutate the map.
- Generated features trace to source data or explicit inference.
- Same data/assets/settings/seed yield deterministic plans.
- Overrides survive replanning; changing asset choices does not redownload geography.
- Stale job results cannot apply to changed documents/georeferences.
- Re-import updates owned generated features without deleting unrelated edits.
- Expensive analysis runs off the UI thread; document mutation remains under editor ownership.
- Cancellation/undo include generated asset files as well as map records, with no broken references.

## 6. Road selection and generation

### Preserve source semantics

Current `Way` retains class, one-way boolean, nodes, and geometry but loses way ID and most tags. Preserve:

- `highway`, `service`, `surface`, `smoothness`;
- lanes, directional counts, lane widths, turn lanes;
- `width`, `est_width`, sidewalk/cycleway/parking information;
- `oneway`, `junction`, access and vehicle/bus restrictions;
- `bridge`, `tunnel`, `layer`, relevant elevation information;
- names, IDs, restriction relations, and confidence/provenance.

Normalize reverse one-way flow consistently with directional attributes. Explicit values override inferred defaults. Keep carriageway width distinct from total profile width.

### Confirmed geometry issues

- `oneway=-1` is ignored.
- `join` can reverse/combine one-way pieces without enforcing compatible directed flow.
- Merging does not preserve width/lane/surface transitions.
- `tunnel=yes` ways are skipped; bridge/layer semantics are not retained.
- Node-based clipping can miss segments crossing the map with both endpoints outside.
- Simplification does not protect semantic junction anchors.
- Geometric proximity is not a substitute for source topology, particularly for parallel or grade-separated roads.

Build graph connectivity from source node identity and vertical/access semantics. Split at junctions and attribute changes; simplify only between protected anchors.

### Asset descriptors

Extend catalogue data with directional vehicle paths, widths/lateral positions, pedestrian/rail paths, actual `[xmin,xmax]` profile bounds, estimated carriageway boundaries, surface/style confidence, dependency health, compatibility, connector behavior, and user corrections.

Maximum absolute profile coordinate is not an accurate carriageway width for asymmetric assets. Path count is not guaranteed to equal painted lane count in every addon.

### Width inference

Preference order:

1. Valid explicit measured width.
2. Lane widths and directional configuration.
3. Lane counts with regional/class priors.
4. Conservative class/context defaults.

Handle sidewalks, parking, shoulders, and median separately. Track measured/interpreted/inferred status. Initial configurable lane-width priors might be 2.75–3.25 m for local streets and 3.25–3.75 m for larger roads, with narrow shared-road cases. These are modeling assumptions, not measured Serbian road dimensions.

### Matching

Hard-filter incompatible mode, broken dependencies, unusable directionality, and unacceptable path configuration. Rank remaining candidates with normalized confidence-weighted errors, initially:

```text
C = 4·lane_error + 3·direction_error + 2·width_error
    + sidewalk_error + surface_error + transition_cost
```

Unknown values reduce confidence rather than automatically excluding assets. Optimize small candidate sets along connected runs with continuity penalties, preserving real transitions. Expose ranked alternatives and reasons, with per-segment/street/rule overrides.

### Junctions and AI

Reuse generation driven by graph data:

- Connect appropriate incoming/outgoing lane endpoints.
- Enforce supported `no_*`/`only_*` restrictions and access rules.
- Preserve roundabout flow and entry/exit structure.
- Avoid accidental U-turns and cross-layer links.
- Validate bus turning envelopes and width transitions.
- Mark unsupported complex configurations for review.

Existing junction paths do not receive an imported restriction model. Acceptance must test connectivity and legal movements, not only visual appearance.

Fallback order: compatible installed junction; validated generated junction; explicit unresolved junction. Prefer installed roads, then existing `RoadSpec` procedural cross-sections. Defer arbitrary bespoke road-surface generation.

## 7. Building selection and generation

### Existing behavior

- Reads closed building ways; height from explicit height, levels, or type guesses.
- Measures candidate model width/depth/height.
- Scores logarithmic dimension ratios plus height.
- Randomizes among near-best candidates.
- Faces nearby roads and compensates horizontal model-bounds centre.

Limitations: no building multipolygon/part composition; semantic type discarded after height inference; ground-plan ratios 0.6–1.7 accepted; centre-only road avoidance; assumed facade direction; no general building overlap resolution; synchronous model waiting.

### Index and source model

Discover candidates across the library, progressively extracting local dimensions/base height, oriented bounds, simplified footprint, possible facade orientations, complexity/LODs, type/style confidence, scripts/conditional geometry, dependency health, and static-placement suitability.

Perform CPU metadata/geometry analysis without loading the whole library onto the GPU. Fingerprint referenced models and dependencies, not just `.sco` modification time. Size alone cannot classify buildings; filename/folder clues remain low-confidence hints with editable corrections.

Support multipolygons/holes, building parts, `height`, `min_height`, levels, roof levels/shape/height, and building/use tags. Avoid double-generating a parent and all its parts. Level-derived height remains an estimate.

### Placement

1. Validate footprint geometry.
2. Derive orientations from footprint edges, oriented bounds, entrances, and frontage.
3. Retrieve a small dimension/type candidate set.
4. Score transformed footprint overlap, overspill, proportions, height, facade direction.
5. Reject road/building/clearance conflicts using full occupancy.
6. Choose stable variation among similarly good candidates.
7. Apply explicit terrain/base-height policy.

Prefer source footprint constraints over blindly facing the nearest road. Do not stretch installed models by default. Use procedural fallback or an unresolved marker when no model fits.

### Procedural sequence

1. Footprint extrusion and flat roof.
2. Simple gabled/hipped roofs for supported shapes.
3. Parts, setbacks, courtyards, facade variation.

Use shared atlases, bounded geometry, LODs, reusable assets, and preserved provenance/editability. Target existing OMSI export formats first; validate in OMSI 2 and openOMSI separately. Optional openOMSI material enhancements come after baseline compatibility.

## 8. External reuse and tradeoffs

| Solution | Role | Recommendation |
|---|---|---|
| Existing pinned openOMSI crates | OMSI formats, geometry, textures, content. | Primary format layer. |
| GeoRust `geo` | Polygon operations, bounds, clipping, buffers. | Strong candidate; benchmark needed algorithms. |
| `rstar` | Spatial index. | Introduce where existing grids are insufficient. |
| `osmpbf` | Rust PBF reading. | Evaluate streaming and bounded-memory extraction. |
| `osmpbfreader` | PBF object/dependency handling. | Compare memory and dependency resolution. |
| Osmium Tool | Extracts, filtering, conversion, fixtures. | Development/advanced-user tool, not mandatory basic-workflow dependency. |
| OSM2World | Mature OSM visual geometry. | Optional building/scenery converter and quality baseline, not OMSI traffic core. |

Research found `geo` MIT/Apache-2.0 licensing; `osmpbfreader` WTFPL with MIT protobuf definitions; the current OSM2World repository identifies MIT licensing and glTF/GLB/OBJ outputs. Pin versions and audit actual dependencies/resources before integration. Do not extrapolate current licensing or features to older releases.

OSM2World can reuse visual-generation work, but does not automatically supply OMSI splines, lane connectivity, restrictions, editable source mapping, or suitable performance budgets. Compare a small prototype before adopting runtime/conversion dependencies.

Prefer existing georeference and pure-Rust geometry initially. GDAL/PROJ/GEOS can be evaluated for demonstrated gaps, weighing native Windows distribution cost.

## 9. Milestones and acceptance

All new module names are proposed. Targets below are future gates, not measured results.

### Phase 0 — Diagnose

- Files: `geo/fetch.rs`, `ui_osm.rs`, logging; network harness.
- Dependency/difficulty: exact failing selection and cache state.
- Acceptance: every attempt reports dataset, endpoint, status, queue/request time, bytes, failure category. Reproduction yields a specific cause or documented remaining uncertainty.

### Phase 1 — Reliable neighborhood acquisition

- Files: fetch/UI/app and small OSM job module.
- Difficulty: shared OSM-map caller and interruptible transport/queue waits.
- Acceptance: roads first; disabled layers make zero requests; valid empty cache; incomplete responses rejected; worker failure exits busy state; retry preserves successes. Cancel acknowledged within 250 ms, work termination target ≤5 seconds.

### Phase 2 — Reusable dataset and local import

- Files: parsers and proposed dataset/cache/source modules.
- Dependencies: Phase 1; IDs, relation assembly, seams, PBF references.
- Acceptance: equivalent normalized features from matching frozen online/local input; zero requests on warm import; resume only missing chunks; no seam duplicates or missing crossing segments.
- Start with saved responses and typed data; add PBF/spatial-cache complexity incrementally.

### Phase 3 — Automatic road assets

- Files: library/assets/OSM/UI/road maker and descriptor/matcher modules.
- Dependencies: retained tags and reliable source data.
- Difficulty: addon profile/path ambiguity and inference confidence.
- Acceptance: mixed widths/configurations within one highway category selected automatically; compatible exact lane/direction candidates win controlled tests; explained fallbacks; unchanged descriptor reuse.

### Phase 4 — Driveable topology

- Files: `osm.rs`, `Editor::build_osm`, junction/cjunction/roads/lanes, timetable network/checker.
- Dependencies: graph semantics and per-segment asset choices.
- Difficulty: restrictions, grade separation, transitions, bus geometry.
- Acceptance: reverse one-way, clipping, T/X junctions, roundabouts, restrictions, bridges pass; no unintended cross-layer links; expected permitted movements connected, prohibited absent; export and driving tests pass in both engines.

### Phase 5 — Automatic buildings

- Files: building parser, library/assets, placement, proposed index/planner.
- Dependencies: dataset, reusable descriptors, road occupancy.
- Difficulty: origins, LODs, facades, irregular shapes.
- Acceptance: automatic discovery, multipolygons preserved, deterministic results, no accepted full-footprint collisions beyond specified tolerance, no silent stretching.

### Phase 6 — Procedural building fallback

- Files: building planner, models, mesh crate, existing asset writers, export.
- Dependencies: validated building footprints and placement.
- Difficulty: roofs/holes, materials, performance.
- Acceptance: supported footprints yield valid geometry within 0.25 m footprint deviation; holes remain empty; resources resolve; representative outputs load in both engines; unsupported shapes explicitly reported.

### Phase 7 — City-scale integration

- Files: job/plan/apply/UI/history/save/check/export.
- Dependencies: preceding stages.
- Difficulty: memory/undo/file lifecycle, stale results, overrides, runtime budgets.
- Acceptance: frozen Kragujevac city area imports offline within agreed hardware budgets; idempotent re-import; overrides survive; cancellation leaves no broken references; performance/compatibility report passes.

Do not delay Phase 1 until the full architecture exists.

## 10. Validation strategy

Offline deterministic fixtures are the CI baseline. Public Overpass availability must not decide CI success.

### Network fixtures

Valid empty/nonempty JSON; 429/502/503/504; malformed query; HTTP 200 remark/missing elements; truncated/malformed/oversized body; slow/stalled reads; cancellation at each stage; corrupt/expired cache; interrupted writes; fallback failure; worker disconnect; competing background-map request.

### Road fixtures

Reverse one-way; one-way ways meeting head-to-head; lane changes; divided and shared narrow roads; both-endpoints-outside crossing; nonrectangular tiles; bridges/tunnels; roundabouts; restrictions; rail-road crossings. Assert graph connectivity and legal movements, not just counts or screenshots.

### Building fixtures

Rectangles, L/U shapes, courtyards, multipolygons, parts, missing heights, units, rotated/off-centre models, slopes, oversized candidates, adjacent repeated buildings.

### Kragujevac study

Freeze exact source date, polygons, asset inventory, settings, and seed for:

1. Original 5×3-tile acquisition case.
2. Mixed residential neighborhood.
3. Larger area covering central streets, industrial/commercial buildings, major roads, river crossings, and peripheral development where present.

Measure tag completeness rather than assume it. Suggested initial review targets, subject to baseline validation:

- ≥90% of 100 reviewed ordinary road segments need no asset override.
- All sampled explicit one-way/supported restrictions respected.
- No unexplained disconnected supported junctions.
- ≥80% of 100 ordinary buildings have an acceptable model or footprint-faithful fallback.
- Every omitted/unresolved feature has a reason.
- Zero network requests for unchanged cached imports.
- Zero duplicates on unchanged re-import.
- Progress refresh at least twice per second during active work.

Measure cold/warm acquisition, CPU planning, peak memory, asset/triangle/material counts, export size, and runtime frame time separately. Choose city time/memory budgets on a named reference machine; no measured baseline currently supports an absolute city-import-time claim.

Reuse existing parser, geometry, editor junction, checker, and gamecheck tests. Explicitly add OMSI 2 runtime validation alongside openOMSI.

## 11. Risks and limits

- Missing source tags require transparent estimates and overrides.
- Addons may contain incorrect paths, unusual origins, missing dependencies, or ambiguous metadata.
- Geometric correctness can exceed OMSI 2 performance budgets: use detail presets, density controls, LODs, and generated-asset reuse.
- Complex interchanges, restrictions, embedded tramways, and unusual junctions need staged support.
- Validate metric scale in both existing georeference modes; make grades/foundations explicit.
- Preserve OSM attribution/source provenance in exports, not just UI.
- Installed addon availability is not redistribution permission, including extracted textures.
- Existing export packaging distinguishes installation/content location; location alone is not a license policy.
- Shared formats/readers do not prove identical runtime behavior in OMSI 2 and openOMSI.

## 12. Research references

Consulted during planning; recheck versions and service policies before implementation:

- [Overpass resource management](https://dev.overpass-api.de/overpass-doc/en/preface/commons.html)
- [Overpass API and instances](https://wiki.openstreetmap.org/wiki/Overpass_API)
- [Private.coffee operator endpoint documentation](https://overpass.private.coffee/)
- [Geofabrik Serbia extract](https://download.geofabrik.de/europe/serbia.html)
- [OSM one-way semantics](https://wiki.openstreetmap.org/wiki/Key:oneway)
- [OSM lane semantics](https://wiki.openstreetmap.org/wiki/Key:lanes)
- [OSM width semantics](https://wiki.openstreetmap.org/wiki/Key:width)
- [OSM attribution and license](https://www.openstreetmap.org/copyright)
- [openOMSI](https://github.com/openOMSI-org/openOMSI)
- [GeoRust geo](https://github.com/georust/geo)
- [rstar](https://github.com/georust/rstar)
- [osmpbf](https://github.com/b-r-u/osmpbf)
- [osmpbfreader](https://github.com/TeXitoi/osmpbfreader-rs)
- [Osmium Tool](https://osmcode.org/osmium-tool/)
- [OSM2World](https://github.com/tordanik/OSM2World)

## 13. Next-agent checklist

1. Read applicable repository instructions and inspect current changes.
2. Verify the findings against current source; do not assume this plan describes a later revision.
3. Confirm the current user request authorizes implementation before making code changes.
4. If implementing, begin with Phase 0/1, preserving existing features and shared callers.
5. Keep confirmed causes, hypotheses, proposed fixes, and measured outcomes distinct.
6. Do not claim live reproductions, successful tests, performance figures, or engine compatibility without evidence.
7. Update this plan's status as milestones are actually completed.

## 14. Implementation update — 2026-11-08

The first Phase 0/1 acquisition changes are implemented. This does not mark the entire overhaul, or all Phase 0/1 acceptance gates, complete.

Implemented:

- `geo/osm_source.rs`: shared cancellable Overpass transport using reqwest/Tokio; the existing `geo/fetch.rs::overpass` entry point remains available to the background map and scripted callers.
- Structured attempt diagnostics record dataset/query ID, endpoint, status, queue/request timing, bytes, content type, and bounded server explanations. The importer logs its document directory, exact tile keys, georeference stamp, tile size, bounding box, and enabled/requested layers.
- Cache lookup and validation precede the network queue, including a second lookup after queue acquisition. Valid empty results are cached, incomplete/malformed results rejected, and writes use a temporary file plus atomic rename.
- `geo/osm_job.rs`: roads/stops first, selected supplementary layers only, independently published successes, per-layer errors, panic/disconnection handling, and retries retaining completed layers.
- Import UI: layer switches before acquisition, elapsed time, diagnostics, Cancel, and retry of missing layers. Build waits for acquisition to finish and all enabled layers to be available. Changing tile identity/georeference clears stale data and cancels the old job.
- Cancellation checkpoints cover queue waits, requests, body reads, fallback backoff, JSON deserialization, and semantic traversal. Closing/discarding a job signals cancellation and prevents late results from being applied.
- The two new UI labels are translated into all 26 translation tables, alongside the existing English/Russian texts.

Current conservative transport defaults are 10 s connect, 95 s per read, 105 s per attempt, and 360 s per interactive job (including queue/backoff). Compatibility calls have a 240 s acquisition budget. These are initial bounds, not timings tuned from the user's live case. Each endpoint is attempted at most once; 429/502/503/504 and transport/incomplete-response failures allow bounded fallback. Numeric Retry-After up to 60 s is honored; longer/date-form guidance is reported without automatic fallback. Query errors stop unchanged retries.

Validation on Windows with an isolated Rust 1.99.1 GNU toolchain (Visual Studio was present, but its Windows SDK libraries were absent):

- `cargo +stable-x86_64-pc-windows-gnu build --release`: passed; existing unrelated warnings remain.
- `cargo +stable-x86_64-pc-windows-gnu test --release osm -- --nocapture`: 22 passed, including transport, coordinator, UI receiver handling, existing OSM parsers/geometry, and background-map rendering.
- `cargo +stable-x86_64-pc-windows-gnu test --release geo::fetch::tests`: 3 passed.
- `python tools/i18n/check.py`: passed, 2,730 texts across 26 translation tables.
- `git diff --check`: passed with normal repository line-ending settings.
- The socket fixture verifies actual connection closure for stalled headers and a stalled body, plus queue-permit release; the two-case test completed in approximately 0.25 s. This is a local fixture result, not a measured UI cancellation latency.
- The full release suite was attempted, then stopped during existing installed-map/city integration checks that scan the local OMSI installation. A rerun excluding the all-installed-maps save check was also stopped during the existing real-city picking tests. The full suite is therefore **not** reported as passing. The stopped all-installed-maps test's temporary copy was removed; original installation data was not changed.

Offline acquisition tests use bounded loopback HTTP servers and never require public Overpass availability. Loopback tests were run outside the sandbox because its networking restrictions prevented local connections. No live public-endpoint reproduction or driving test was performed.

Still outstanding:

- Reproduce the original exact 5×3 selection, including cold/warm cache and background-map comparisons. Its original live failure has not been reproduced; no specific endpoint is claimed as its cause.
- Measure interactive cancellation acknowledgement and full worker termination on representative real datasets; local fixtures alone do not establish an end-to-end UI/OS/engine guarantee. Existing road derivation and map generation are still owned by the editor/UI path and are outside this acquisition change.
- Saved-response/local-file UI, configurable endpoints, adaptive chunks, persistent resumable manifests, and PBF input remain later work (beginning Phase 2).
- No OMSI 2/openOMSI driving or export compatibility result is claimed for this acquisition change.

## 15. Automatic roads/buildings implementation — 2026-11-08

The user authorized the next road/building implementation. The first usable automatic planning
path is implemented in `src/osm_auto/`, with UI/app hooks and reuse of existing generation and
format readers. See [OSM_AUTOMATIC_IMPORT.md](OSM_AUTOMATIC_IMPORT.md) for workflow and limits.
This is incremental Phase 2/3/5/6 work, not completion of every milestone acceptance gate.

Implemented:

- Saved Overpass JSON can be loaded without network requests. Mixed inputs are filtered by
  feature tags so unrelated nodes/polygons do not become stops, trees, buildings, or water.
- Roads retain IDs/tags/restrictions, normalize reverse one-way flow, preserve tag transitions,
  and clip against the actual tile union. Per-road asset choices reach `Editor::build_osm`.
- A cancellable background worker builds a CPU asset index with dependency fingerprints,
  ranks installed road splines, measures building models, and returns an inspectable plan.
  Roads-only planning skips building catalogue/occupancy work.
- Road ranking uses directional paths, carriageway width, sidewalks, surface hints, and mode;
  optional procedural fallback uses `RoadSpec`, including a bidirectional shared-lane option.
- Automatic building discovery uses conservative name/folder hints plus measured geometry.
  Selection checks origins, dimensions, height, orientation, footprint coverage, and occupancy.
- Building multipolygons/holes, parts, height units, and minimum height are retained. Supported
  unmatched footprints receive editable flat-roof extrusions and shared generated atlases.
- Preview alternatives and per-road choices persist during replanning in the current session.
  Incompatible overrides, changed documents, and changed descriptor dependencies are rejected.
- Apply prepares/validates files before mutating the map, saves a source/choice report, and
  merges asset-file and map-record history so undo/redo restores both. Preparation failures
  roll back tracked resources; a failed undo resource restore preserves the original step.
- Existing manual class/folder mapping remains available when automatic mode is disabled.
- New UI labels are translated in all 26 locale tables.

Important outstanding gates:

- Source-graph connectivity and supported restriction enforcement in junction generation remain
  Phase 4. The automatic planner currently omits and reports grade-separated/turn-restricted
  roads rather than claiming supported driveable topology for them.
- Addon classification, carriageway estimates, and conservative footprint bounds still need
  evaluation against the installed inventory and frozen Kragujevac samples. Architectural style,
  frontage/facade interpretation, arbitrary origins, and complex building parts need refinement.
- PBF/XML adapters, overlapping spatial cache/chunk resumes, persistent overrides across app
  sessions, owned-feature re-import reconciliation, and agreed city-scale budgets remain open.
- Pitched/complex roofs, foundation geometry, advanced LOD/performance work, and both-engine
  runtime/driving validation remain open. No game-runtime compatibility/performance claim is made.

Validation of this increment (Windows, isolated Rust 1.99.1 GNU toolchain):

- `cargo +stable-x86_64-pc-windows-gnu test --release osm -- --nocapture`: **34 passed**.
- `cargo +stable-x86_64-pc-windows-gnu test --release history::tests`: **5 passed**.
- `cargo +stable-x86_64-pc-windows-gnu test --release road_maker::tests`: **7 passed**.
- `cargo +stable-x86_64-pc-windows-gnu build --release`: **passed**; existing unrelated warnings remain.
- `python tools/i18n/check.py`: **passed**, 2,735 texts across 26 translation tables.
- `git diff --check`: **passed** with normal repository line-ending settings.

The 46 focused tests cover installed/procedural selection and file writing, actual application
through the existing road builder, combined map/resource undo and redo, failed-resource undo
preserving the original step, model-dependency cache invalidation, offset building origins,
footprint collisions, courtyard roof area, reverse one-way flow, width transitions, sparse tile
clipping, and separation of feature types in saved mixed responses. The HTTP fixtures ran
outside the sandbox because loopback connections are restricted inside it. These results do
not substitute for a full installed-content suite, interactive visual QA, frozen Kragujevac
review, or either engine's runtime/driving checks.

## 16. Preview performance and progress follow-up (2026-11-08)

The installed-content preview exposed an unbounded-looking `n / n+1` building counter.
It counted accepted placements rather than processed source features. It also tested each
building candidate against the complete accumulated occupancy set. These are corrected:

- A fixed source total and processed count now include rejected and off-tile features. The UI
  names the phase, shows its percentage, and estimates remaining time for that phase.
- A 32 m occupancy grid selects nearby obstacles for both preview and preparation. A sorted
  model-area catalogue and dimension filter reject unsuitable models before geometry checks.
- Exact boundary interval tests replace sampled footprint containment and catch narrow notches.
  Cancellation is checked inside model matching. Shared dependency metadata is read once per
  catalogue build rather than once per referring asset.
- The opt-in, read-only `replay_cached_preview` test loads a saved map, source JSON, and cached
  descriptors without writing map data or generated assets.

The real Kragujevac replay used 20 tiles, 7,309 source buildings, 374 prepared roads,
9,003 indexed building models, and 8,607 road assets. Planning took 1.006 s with separate
cached responses and 0.817 s with the combined cached JSON; both produced 324 road choices,
3,424 building choices, and 3,885 skipped source buildings. These measurements exclude
catalogue loading/fingerprint checking, generated-file preparation, rendering, and engine runs.

Validation: the optimized OSM suite passed 39 tests, with the real-data replay ignored by
default; the opt-in combined replay also passed. The release executable was rebuilt and
installed at `target/release/openmapper.exe`. Cached roads, buildings, vegetation, water, and
bus routes were combined into `target/imports/Kragujevac-cached.osm.json` (8,557 unique
elements), which the same replay reads successfully. The input cache and saved map were
not modified.

## 17. Serbian interface (2026-11-08)

Serbian Latin (`sr`, **Srpski (latinica)**) is available in View > Language, Settings, and
command search. Locale spellings including `sr_RS` and `sr_Latn_RS` resolve to it; the
existing preferences persist its code. The Latin interface font already covers its script.

`locales/sr.tsv` contains all 2,737 current UI texts, including importer controls and errors.
This is an initial machine-assisted translation with 393 entries reviewed for interface,
road, building, and timetable terminology. Broader native-speaker review remains useful.
Technical placeholders, product names, file names, and shortcuts are preserved.

Validation: all six i18n tests passed, including Serbian locale lookup, table loading,
importer terminology, and technical-token preservation. The translation checker passes
for all 2,737 texts across 27 locale tables (29 interface languages with inline English
and Russian).

The Serbian release build passed and was copied to `target/release/openmapper.exe` with
matching SHA-256 hashes after confirming the editor was closed. The final table has no
missing entries, empty translations, temporary service markers, or placeholder mismatches;
`git diff --check` passes with the repository's normal Windows line-ending settings.

## 18. Terrain-review failures no longer abort a city import (2026-11-08)

A user build stopped at `Building way/... needs a terrain/foundation review`. The writer
returned an error when any footprint crossed more than 2 m of terrain variation, rolling
back the prepared resources and leaving the whole map unchanged. Such buildings now remain
unbuilt for review while roads and supported buildings continue. Their source IDs and
measured variation are logged, returned as preparation warnings, and saved in the manifest.
The import panel retains the completion summary and a bounded diagnostics list after the
temporary toast disappears. Other preparation errors remain transactional, retain the
preview for inspection/retry, and are now logged and shown persistently in the panel.

The mixed installed/procedural application regression now includes a third building on a
steep second tile: preparation must retain both roads and both supported buildings, report
the steep building, write that warning to the manifest, and preserve combined undo/redo.
The opt-in cached replay also checks read-only build preflight, beyond planning alone.

Preview currently means an inspectable choices list; a map overlay is not implemented.

Validation: 39 OSM tests passed (one opt-in replay ignored), including the expanded steep-terrain
application/undo regression; translation completeness and whitespace checks passed. The release
build passed and `target/release/openmapper.exe` was updated and SHA-256 verified. Interactive
inspection was unavailable because the computer-use runtime could not initialize; the user
provided the exact error text. The saved user map was not modified during diagnosis or testing.
