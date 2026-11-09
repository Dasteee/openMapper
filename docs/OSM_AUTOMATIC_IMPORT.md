# Automatic OSM roads and buildings

This is the first automatic asset-planning implementation. It supplements the existing
manual importer; it does not complete the city-scale overhaul or its driving-validation gates.

## Using it

1. Place/georeference the map and load terrain heights as usual.
2. Open **Map → From OpenStreetMap**. Select the supplementary layers you want, including
   **Buildings**. Download, or use **Load OSM JSON** for a saved Overpass JSON response.
3. Leave **Automatic roads and buildings** enabled. **Procedural fallback** permits generated
   assets when the installed library cannot provide a suitable match.
4. Select the road classes to include. Class and one-way asset choices act as optional
   overrides. A building folder selection restricts the building candidate pool; leave the
   folder list empty for automatic discovery.
5. Click **Preview automatic import** after the catalogue scan finishes. The background worker
   indexes assets and prepares a proposal. It does not build anything in the map.
6. Expand **Preview** to inspect selected assets, reasons, alternatives, and unresolved features.
   Road alternatives can be changed per prepared road. **Reset** clears those segment choices.
7. Click **Build**. Existing stop and vegetation requirements still apply: select a stop object
   or untick stops, and choose a plant mix or untick vegetation.

Changing tiles, georeferencing, enabled building import, class choices, or document revisions
invalidates the preview. Building also checks the document and measured asset dependencies.
Replanning and changing asset choices reuse downloaded geography.

## Roads

- OSM way IDs, node IDs, tags, reverse one-way direction, and restriction references are retained.
- Road preparation clips against the actual selected tile union, including crossing segments
  with both original endpoints outside the selection.
- Joins preserve tag changes and do not reverse incompatible one-way flow.
- Matching compares directional vehicle paths, estimated carriageway width, sidewalks, surface
  hints, and mode. Actual lateral profile bounds are stored separately from carriageway width.
- Missing lane/width data uses conservative class priors and is identified as estimated.
- Compatible installed splines are preferred. Procedural fallback uses the existing road maker,
  with directional paths, shared narrow-road paths, asymmetric sidewalks, kerbs, surface
  textures, and default markings where applicable. Rail fallback is not generated.
- Construction still uses the editor's existing road and junction builder.

Grade-separated roads and ways carrying retained turn restrictions are currently reported as
unresolved and omitted by the automatic planner. Full source-graph junction connectivity,
restriction-aware movements, complex transitions, and bus turning-envelope validation remain
Phase 4 work. Manual mode retains its existing behavior.

## Buildings

- Building multipolygons, courtyards, parts, IDs, tags, height, and minimum height are retained.
  Parent envelopes are suppressed when explicit parts describe the building.
- The worker searches installed scenery using building/name/folder hints, then measures models
  on the CPU. Hints have limited semantic precision; they do not establish architectural style.
- Scripted, animated, conditional, technical, and broken candidate assets are excluded.
- Candidate footprints are tested at orientations derived from the source footprint. Matching
  accounts for model origin offsets, height, coverage, and occupancy. Models are not stretched.
- Occupancy includes existing roads/objects, the planned road curves and junction reservations,
  and previously accepted buildings. Bounds provide a conservative approximation of installed
  model footprints; facade/entrance interpretation remains future work.
- If no installed model fits, supported footprints receive an editable extrusion with a flat
  roof, preserved courtyard holes, collision geometry, and shared generated texture atlases.
- Placement uses a level base on the highest sampled footprint corner. More than two metres
  of terrain variation requires review: that building is skipped while roads and other buildings
  continue. Skipped source IDs and measured height differences remain in Import diagnostics,
  the log, and `openMapper/osm_plan.json`; the completion summary remains in the import panel.
  Complex slopes, foundations, pitched roofs, and
  vertically overlapping building parts need further work.

## Files, caching, and undo

Preview progress reports the catalogue, objects, roads, and buildings separately. Each feature
phase counts every processed source feature, including rejected and off-tile features, against
a fixed source total. That total can exceed the map-filtered layer count. The estimated remaining
time is approximate and applies to the current phase. A spatial occupancy grid and a size-sorted
building catalogue keep model matching local instead of scanning every obstacle for every model.

- Descriptors are cached in the content root's `openMapper/osm_asset_index_v1.json`. Referenced
  model and texture file metadata participate in invalidation. A roads-only preview skips
  building indexing and building occupancy analysis.
- Generated roads live under `Splines/openMapper/`; generated buildings under
  `Sceneryobjects/openMapper/OSM/`. They use the existing `.sli`, `.sco`, `.x`, road-spec, and
  editable-source writers.
- `openMapper/osm_plan.json` in the map records source IDs/tags, choices, reasons, and unresolved
  features, with OSM attribution. It is a planning report, not a complete generated-record
  ownership/re-import database.
- File preparation is checked before map mutation. A preparation error restores tracked
  files. Generated files and map records participate in the import's merged undo/redo step.
  Failed resource restoration leaves the map and history step intact.

Saved input currently means Overpass JSON with `elements` and inline geometry. XML/PBF,
spatial chunk reuse, resumable manifests, persistent override management across sessions,
idempotent owned-feature updates, and city-scale budgets remain later milestones.

Generated formats are read back through the existing openOMSI-backed readers in offline
fixtures. This does not constitute OMSI 2/openOMSI driving, visual, or performance validation.
