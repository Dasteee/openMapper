//! Where a map lies on the Earth, and the satellite imagery and terrain heights fetched for
//! it. All of it is the editor's own: the placement and the per-tile settings are kept in
//! `<map>/openMapper/geo.json`, each tile's imagery in `<map>/openMapper/imagery`, and the
//! game reads none of it (only the terrain heights written into the tiles are the map's).

pub mod aerial;
pub mod elevation;
pub mod fetch;
pub mod imagery;
pub mod job;
pub mod own;
pub mod osm_source;
pub mod osm_job;
pub mod sources;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::DVec2;
use serde::{Deserialize, Serialize};

use crate::doc::TileKey;

// WGS84, the ellipsoid the imagery's latitudes and longitudes are given on
const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;
const E2: f64 = F * (2.0 - F);

/// Radius of curvature along the meridian at latitude `phi` (radians).
fn meridian_radius(phi: f64) -> f64 {
    let s = phi.sin();
    A * (1.0 - E2) / (1.0 - E2 * s * s).powf(1.5)
}

/// Radius of curvature across the meridian (the parallel's radius is this times cos phi).
fn prime_radius(phi: f64) -> f64 {
    let s = phi.sin();
    A / (1.0 - E2 * s * s).sqrt()
}

/// Web Mercator (the tiles' projection) of a latitude and longitude: x east and y south,
/// 0..1 over the whole world.
pub fn merc_of(lat: f64, lon: f64) -> DVec2 {
    let phi = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    DVec2::new((lon + 180.0) / 360.0, (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / std::f64::consts::PI) / 2.0)
}

/// The inverse of [`merc_of`]: (latitude, longitude) in degrees.
pub fn lat_lon_of_merc(m: DVec2) -> (f64, f64) {
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * m.y)).sinh().atan().to_degrees();
    (lat, m.x * 360.0 - 180.0)
}

/// Metres on the ground a Web Mercator pixel of zoom `z` covers at a latitude.
pub fn metres_per_pixel(lat: f64, z: u8) -> f64 {
    40_075_016.686 * lat.to_radians().cos() / (256.0 * (1u64 << z) as f64)
}

/// Where a plain map's anchor tile lies: the latitude and longitude of its middle, and how
/// far the map's north is turned from the true north (degrees, clockwise).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub tile: [i32; 2],
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub rotation: f64,
}

/// How the map's world metres turn into latitude and longitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Georef {
    /// A `[worldcoordinates]` map: tile (x, y) is the Web Mercator tile (x + 32768,
    /// 32767 - y) of zoom 16 (see `omsi_map::tile_local_to_lat_lon`), `ts` grid metres a
    /// side, so the map's metres are the Mercator's, scaled.
    World { ts: f64 },
    /// A plain map placed by hand: world point `at` lies at (lat, lon), and the map is
    /// turned by `rot` degrees. Metres are true ground metres along the meridians and the
    /// parallels (WGS84), so a road drawn 300 m long is 300 m long in the imagery too; the
    /// mapping is one smooth function of the world position, so tiles cannot part at their
    /// edges however far from the anchor they are.
    Local { at: DVec2, lat: f64, lon: f64, rot: f64 },
}

impl Georef {
    pub fn local(a: &Anchor, ts: f64) -> Georef {
        let at = DVec2::new((a.tile[0] as f64 + 0.5) * ts, (a.tile[1] as f64 + 0.5) * ts);
        Georef::Local { at, lat: a.lat, lon: a.lon, rot: a.rotation }
    }

    /// Latitude and longitude (degrees) of a world point.
    pub fn lat_lon(&self, p: DVec2) -> (f64, f64) {
        match *self {
            Georef::World { .. } => lat_lon_of_merc(self.merc(p)),
            Georef::Local { at, lat, lon, rot } => {
                let d = p - at;
                let (s, c) = rot.to_radians().sin_cos();
                let (e, n) = (d.x * c + d.y * s, -d.x * s + d.y * c);
                let phi0 = lat.to_radians();
                // (the meridian arc over a few km: its radius at the middle latitude)
                let mut phi = phi0 + n / meridian_radius(phi0);
                for _ in 0..4 {
                    phi = phi0 + n / meridian_radius((phi0 + phi) * 0.5);
                }
                let lam = e / (prime_radius(phi) * phi.cos());
                (phi.to_degrees(), lon + lam.to_degrees())
            }
        }
    }

    /// Web Mercator position (0..1, y south) of a world point.
    pub fn merc(&self, p: DVec2) -> DVec2 {
        match *self {
            Georef::World { ts } => DVec2::new((p.x / ts + 32768.0) / 65536.0, (32768.0 - p.y / ts) / 65536.0),
            Georef::Local { .. } => {
                let (la, lo) = self.lat_lon(p);
                merc_of(la, lo)
            }
        }
    }

    /// The world point at a latitude and longitude (the inverse of [`Georef::lat_lon`]).
    pub fn world(&self, la: f64, lo: f64) -> DVec2 {
        match *self {
            Georef::World { ts } => {
                let m = merc_of(la, lo);
                DVec2::new((m.x * 65536.0 - 32768.0) * ts, (32768.0 - m.y * 65536.0) * ts)
            }
            Georef::Local { at, lat, lon, rot } => {
                let (phi0, phi) = (lat.to_radians(), la.to_radians());
                let n = (phi - phi0) * meridian_radius((phi0 + phi) * 0.5);
                let e = (lo - lon).to_radians() * prime_radius(phi) * phi.cos();
                let (s, c) = rot.to_radians().sin_cos();
                at + DVec2::new(e * c - n * s, e * s + n * c)
            }
        }
    }

    /// A number that changes when the placement does (imagery made for another one is
    /// out of date).
    pub fn stamp(&self) -> u64 {
        let v: [f64; 5] = match *self {
            Georef::World { ts } => [ts, 0.0, 0.0, 0.0, 1.0],
            Georef::Local { at, lat, lon, rot } => [at.x, at.y, lat, lon, rot],
        };
        v.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| (h ^ x.to_bits()).wrapping_mul(0x100_0000_01b3))
    }
}

/// One tile's overlay and what was fetched for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileGeo {
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub imagery: Option<ImageryMeta>,
    /// The terrain heights were loaded (the source's name).
    #[serde(default)]
    pub terrain: Option<String>,
    /// The user's own picture shown here (its id in [`GeoFile::pictures`]) in place of the
    /// downloaded imagery.
    #[serde(default)]
    pub own: Option<String>,
}

impl Default for TileGeo {
    fn default() -> Self {
        TileGeo { visible: true, opacity: 1.0, imagery: None, terrain: None, own: None }
    }
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// What a tile's imagery file was made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageryMeta {
    pub source: String,
    pub zoom: u8,
    /// [`Georef::stamp`] of the placement it was made for.
    pub stamp: u64,
    /// Some of it came from a lower zoom (the source has nothing finer there).
    #[serde(default)]
    pub coarser: bool,
    /// The picture is kept in the map's folder. Google's terms allow no lasting copy: its
    /// pictures are made again from the tile cache each time the map is opened.
    #[serde(default = "yes")]
    pub kept: bool,
}

/// Terrain loading options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainOpts {
    /// The anchor tile's mean height is taken off, so the map lies near 0 m.
    #[serde(default = "yes")]
    pub relative: bool,
    /// Added to every height (m).
    #[serde(default)]
    pub offset: f32,
    /// Smoothing radius (m; 0: none).
    #[serde(default)]
    pub smooth: f32,
    /// The height taken off with `relative`, fixed by the first load so that later tiles
    /// fit the earlier ones.
    #[serde(default)]
    pub base: Option<f32>,
}

impl Default for TerrainOpts {
    fn default() -> Self {
        TerrainOpts { relative: true, offset: 0.0, smooth: 0.0, base: None }
    }
}

/// `<map>/openMapper/geo.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoFile {
    /// Where a plain map lies (None: not placed yet; a `[worldcoordinates]` map needs none).
    #[serde(default)]
    pub anchor: Option<Anchor>,
    /// The imagery source chosen (its name).
    #[serde(default)]
    pub source: String,
    /// The zoom of the imagery (0: the finest the source has here).
    #[serde(default)]
    pub zoom: u8,
    /// Edge of each tile's image (px; 0: as fine as the source is there).
    #[serde(default)]
    pub size: u32,
    /// Tiles added later get their imagery by themselves.
    #[serde(default = "yes")]
    pub auto_new: bool,
    /// Terrain heights are loaded with the imagery.
    #[serde(default)]
    pub with_terrain: bool,
    #[serde(default)]
    pub terrain: TerrainOpts,
    /// The overlay as a whole.
    #[serde(default = "yes")]
    pub overlay: bool,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Put the imagery into zips exported for sharing (its licence may not allow that).
    #[serde(default)]
    pub export_imagery: bool,
    /// Keep the photos of only this many tiles nearest the camera (None: every tile that has
    /// one). A map's own aerial photos are hundreds of megabytes: a window of them is enough
    /// to see where one is, and the rest are not held in the graphics card's memory.
    #[serde(default)]
    pub photos_near: Option<u32>,
    /// Per tile ("x_y").
    #[serde(default)]
    pub tiles: BTreeMap<String, TileGeo>,
    /// The user's own pictures by id.
    #[serde(default)]
    pub pictures: BTreeMap<String, own::Picture>,
}

impl Default for GeoFile {
    fn default() -> Self {
        GeoFile { anchor: None, source: String::new(), zoom: 0, size: 0, auto_new: true, with_terrain: false, terrain: Default::default(), overlay: true, opacity: 1.0, export_imagery: false, photos_near: None, tiles: BTreeMap::new(), pictures: BTreeMap::new() }
    }
}

pub fn tile_name(k: TileKey) -> String {
    format!("{}_{}", k.0, k.1)
}

/// The editor's folder in a map's folder.
pub fn dir_of(map_dir: &Path) -> PathBuf {
    map_dir.join("openMapper")
}

pub fn imagery_path(map_dir: &Path, k: TileKey) -> PathBuf {
    dir_of(map_dir).join("imagery").join(format!("tile_{}_{}.jpg", k.0, k.1))
}

/// A tile's piece of the user's own picture.
pub fn own_path(map_dir: &Path, k: TileKey) -> PathBuf {
    dir_of(map_dir).join("imagery").join(format!("tile_{}_{}.png", k.0, k.1))
}

impl GeoFile {
    pub fn path(map_dir: &Path) -> PathBuf {
        dir_of(map_dir).join("geo.json")
    }

    pub fn load(map_dir: &Path) -> GeoFile {
        std::fs::read_to_string(Self::path(map_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, map_dir: &Path) -> std::io::Result<()> {
        let p = Self::path(map_dir);
        std::fs::create_dir_all(p.parent().unwrap())?;
        let tmp = p.with_extension("json.openmapper-new");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, p)
    }

    pub fn tile(&self, k: TileKey) -> TileGeo {
        self.tiles.get(&tile_name(k)).cloned().unwrap_or_default()
    }

    pub fn tile_mut(&mut self, k: TileKey) -> &mut TileGeo {
        self.tiles.entry(tile_name(k)).or_default()
    }

    /// How the map lies on the Earth, if it does.
    pub fn georef(&self, world_coordinates: bool, ts: f64) -> Option<Georef> {
        if world_coordinates {
            return Some(Georef::World { ts });
        }
        self.anchor.as_ref().map(|a| Georef::local(a, ts))
    }
}

/// Leave the map's imagery out of an export unless the map says otherwise (the imagery's
/// licence is the source's, not the map maker's). `files`: zip path → file.
pub fn export_filter(map_dir: &Path, files: &mut BTreeMap<String, PathBuf>) {
    if GeoFile::load(map_dir).export_imagery {
        return;
    }
    let dir = dir_of(map_dir).join("imagery");
    files.retain(|_, p| !p.starts_with(&dir));
}
