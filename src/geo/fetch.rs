//! Downloads: map tiles through a disk cache (never fetched twice), Google's sessions, place
//! search. Everything here blocks and runs on background threads; a few requests at a time,
//! each with openMapper's own User-Agent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::sources::{self, GeoPrefs, Kind, Source};
use crate::i18n::tr;

pub const USER_AGENT: &str = concat!("openMapper/", env!("CARGO_PKG_VERSION"), " (+https://openomsi.org)");

/// Requests on the network at once (all downloads together).
const PARALLEL: usize = 4;

fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| ureq::Agent::config_builder().user_agent(USER_AGENT).timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into())
}

/// A slot on the network (at most [`PARALLEL`] are out).
struct Slot;

static SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

impl Slot {
    fn take() -> Slot {
        let (m, cv) = &SLOTS;
        let mut n = m.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= PARALLEL {
            n = cv.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        *n += 1;
        Slot
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let (m, cv) = &SLOTS;
        {
            let mut n = m.lock().unwrap_or_else(|e| e.into_inner());
            *n = n.saturating_sub(1);
        }
        cv.notify_one();
    }
}

/// Where downloaded tiles are kept: `~/.openomsi/cache/openmapper/tiles/<source>/z/x/y`.
pub fn cache_root() -> PathBuf {
    if let Some(d) = std::env::var_os("OPENMAPPER_TILE_CACHE") {
        return PathBuf::from(d);
    }
    crate::install::openomsi_data_dir().join("cache").join("openmapper").join("tiles")
}

/// A tile as the server gave it.
#[derive(Debug, Clone)]
pub enum Tile {
    /// The picture's file (JPEG or PNG).
    Data(std::sync::Arc<Vec<u8>>),
    /// The source has nothing there at this zoom.
    Missing,
}

/// Why a download failed, in words for the user.
pub type Error = String;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A server's "no imagery here" picture: one flat grey with at most a line of text on it
/// (Esri's "Map data not yet available") or empty. Water is flat too but has colour.
pub fn is_placeholder(bytes: &[u8]) -> bool {
    // (Esri's, byte for byte the same everywhere)
    if bytes.len() == 2521 && fnv(bytes) == 0x484a_b5da_02f7_864f {
        return true;
    }
    let Ok(img) = image::load_from_memory(bytes) else { return true };
    let img = img.to_rgb8();
    let n = (img.width() * img.height()) as usize;
    if n == 0 {
        return true;
    }
    let mut lum: Vec<u8> = Vec::with_capacity(n);
    let mut chroma = 0u64;
    for p in img.pixels() {
        let [r, g, b] = p.0;
        lum.push(((r as u32 * 3 + g as u32 * 6 + b as u32) / 10) as u8);
        chroma += (r.max(g).max(b) - r.min(g).min(b)) as u64;
    }
    let mut sorted = lum.clone();
    sorted.sort_unstable();
    let med = sorted[n / 2] as i32;
    let flat = lum.iter().filter(|l| (**l as i32 - med).abs() <= 10).count() as f64 / n as f64;
    flat > 0.95 && (chroma as f64 / n as f64) < 10.0
}

pub fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, x| (h ^ *x as u64).wrapping_mul(0x100_0000_01b3))
}

/// One tile of a source (from the disk cache when it is there).
pub fn tile(src: &Source, prefs: &GeoPrefs, z: u8, x: u32, y: u32) -> Result<Tile, Error> {
    let dir = cache_root().join(src.slug()).join(z.to_string()).join(x.to_string());
    let data = dir.join(format!("{y}.tile"));
    let none = dir.join(format!("{y}.none"));
    let expiry = dir.join(format!("{y}.until"));
    // (Google's tiles may be kept only as long as their answer says)
    let fresh = src.kind != Kind::Google || std::fs::read_to_string(&expiry).ok().and_then(|s| s.trim().parse::<u64>().ok()).map(|t| t > now_secs()).unwrap_or(false);
    if fresh {
        if let Ok(b) = std::fs::read(&data) {
            return Ok(Tile::Data(std::sync::Arc::new(b)));
        }
        if none.is_file() {
            return Ok(Tile::Missing);
        }
    }
    let key = prefs.key(&src.name);
    if src.needs_key && key.is_none() {
        return Err(format!("{}: {}", src.name, tr("no key entered", "не введён ключ")));
    }
    // (a key never shows in a message)
    let scrub = |e: Error| match key {
        Some(k) => e.replace(k, "…"),
        None => e,
    };
    let session = if src.kind == Kind::Google { Some(google_session(key.unwrap_or("")).map_err(scrub)?) } else { None };
    let url = src.tile_url(z, x, y, key, session.as_deref());
    let (status, body, max_age) = get(&url).map_err(scrub)?;
    let _ = std::fs::create_dir_all(&dir);
    let keep = |p: &PathBuf, b: &[u8]| {
        let tmp = p.with_extension("part");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    };
    if src.kind == Kind::Google {
        keep(&expiry, (now_secs() + max_age.unwrap_or(3600)).to_string().as_bytes());
    }
    match status {
        200 if !body.is_empty() && !(src.kind != Kind::Terrarium && is_placeholder(&body)) => {
            keep(&data, &body);
            Ok(Tile::Data(std::sync::Arc::new(body)))
        }
        200 | 204 | 404 => {
            keep(&none, b"");
            Ok(Tile::Missing)
        }
        401 | 403 => Err(format!("{}: {} (HTTP {status})", src.name, tr("the server refused the key", "сервер не принял ключ"))),
        429 => Err(format!("{}: {}", src.name, tr("too many requests, try again later", "слишком много запросов, попробуйте позже"))),
        _ => Err(format!("{}: HTTP {status}", src.name)),
    }
}

/// GET a URL: (status, body, the answer's max-age). A dropped connection, a timeout or a busy
/// server (429, 5xx) is tried again twice, a little later each time.
fn get(url: &str) -> Result<(u16, Vec<u8>, Option<u64>), Error> {
    let mut last = None;
    for attempt in 0..3u64 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * attempt * attempt));
        }
        let got = get_once(url);
        let retry = match &got {
            Ok((status, _, _)) => matches!(*status, 429 | 500 | 502 | 503 | 504),
            Err(_) => true,
        };
        if !retry {
            return got;
        }
        log::debug!("download attempt {} failed: {}", attempt + 1, got.as_ref().map(|r| r.0.to_string()).unwrap_or_else(|e| e.clone()));
        last = Some(got);
    }
    last.unwrap_or_else(|| Err("no answer".into()))
}

fn get_once(url: &str) -> Result<(u16, Vec<u8>, Option<u64>), Error> {
    // (the slot is held for the one request only, not for the pause between tries)
    let _slot = Slot::take();
    let mut r = agent().get(url).call().map_err(net_error)?;
    let status = r.status().as_u16();
    let max_age = r.headers().get("cache-control").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').find_map(|p| p.trim().strip_prefix("max-age=").and_then(|n| n.parse().ok())));
    let body = r.body_mut().with_config().limit(32 << 20).read_to_vec().map_err(net_error)?;
    Ok((status, body, max_age))
}

fn net_error(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => format!("{} ({e})", tr("No connection to the server: are you online?", "Нет связи с сервером: есть ли интернет?")),
        e => e.to_string(),
    }
}

/// A Google Map Tiles API session for a key: made once, kept until it runs out (also on
/// disk, by a hash of the key, never the key itself).
fn google_session(key: &str) -> Result<String, Error> {
    static SESSIONS: Mutex<Option<HashMap<u64, sources::GoogleSession>>> = Mutex::new(None);
    let h = fnv(key.as_bytes());
    let file = cache_root().join(sources::GOOGLE_SESSION_DIR).join(format!("{h:016x}.json"));
    let mut lock = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    let map = lock.get_or_insert_with(HashMap::new);
    let alive = |s: &sources::GoogleSession| s.expiry > now_secs() + 600;
    if let Some(s) = map.get(&h).filter(|s| alive(s)) {
        return Ok(s.token.clone());
    }
    if let Some(s) = std::fs::read_to_string(&file).ok().and_then(|b| sources::parse_google_session(&b).ok()).filter(alive) {
        map.insert(h, s.clone());
        return Ok(s.token);
    }
    let _slot = Slot::take();
    let url = format!("https://tile.googleapis.com/v1/createSession?key={key}");
    let mut r = agent()
        .post(&url)
        .header("Content-Type", "application/json")
        .send(r#"{"mapType":"satellite","language":"en-US","region":"US"}"#)
        .map_err(net_error)?;
    let status = r.status().as_u16();
    let body = r.body_mut().read_to_string().map_err(net_error)?;
    let s = sources::parse_google_session(&body).map_err(|e| google_refusal(status, &e))?;
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    let _ = std::fs::write(&file, &body);
    map.insert(h, s.clone());
    Ok(s.token)
}

/// Google's refusal in words. Accounts billed in the European Economic Area get no satellite
/// tiles from Google at all, whatever the key: the answer says so, and the other sources work.
fn google_refusal(status: u16, message: &str) -> Error {
    if status == 403 && message.contains("not available for your account and region") {
        return tr(
            "Google (HTTP 403): Google gives no satellite imagery to accounts billed in the European Economic Area, so this key cannot work. Use another source (Esri, Bing, Mapbox, MapTiler), or a key from a Google account billed outside the EEA.",
            "Google (HTTP 403): Google не отдаёт спутниковые снимки аккаунтам с платёжным адресом в Европейской экономической зоне, поэтому этот ключ работать не может. Выберите другой источник (Esri, Bing, Mapbox, MapTiler) или ключ от аккаунта Google вне ЕЭЗ.",
        )
        .to_string();
    }
    format!("Google (HTTP {status}): {message}")
}

/// Google's copyright line and finest zoom for an area around (lat, lon).
pub fn google_viewport(prefs: &GeoPrefs, lat: f64, lon: f64, zoom: u8) -> Result<sources::Viewport, Error> {
    let key = prefs.key(sources::GOOGLE).ok_or_else(|| tr("Enter your Google key first", "Сначала введите ключ Google").to_string())?;
    let session = google_session(key).map_err(|e| e.replace(key, "…"))?;
    let d = 0.01;
    let url = format!("https://tile.googleapis.com/tile/v1/viewport?session={session}&key={key}&zoom={zoom}&north={}&south={}&east={}&west={}", lat + d, lat - d, lon + d, lon - d);
    let (status, body, _) = get(&url).map_err(|e| e.replace(key, "…"))?;
    if status != 200 {
        return Err(format!("Google: HTTP {status}"));
    }
    sources::parse_google_viewport(&String::from_utf8_lossy(&body), lat, lon)
}

/// The finest zoom a source really has at (lat, lon): the highest at which its tile there
/// is a picture and not a placeholder (tried from the finest worth asking for down; the
/// answers stay in the tile cache, so asking again costs nothing).
pub fn finest_zoom(src: &Source, prefs: &GeoPrefs, lat: f64, lon: f64) -> Result<Option<u8>, Error> {
    if !src.covers(lat, lon) {
        return Ok(None);
    }
    let top = src.useful_zoom(lat);
    if src.kind == Kind::OsmMap {
        return Ok(Some(top));
    }
    if src.kind == Kind::Google {
        return Ok(google_viewport(prefs, lat, lon, 16)?.max_zoom.map(|z| z.min(top)));
    }
    let m = super::merc_of(lat, lon);
    let mut z = top;
    while z >= 10 {
        let n = (1u64 << z) as f64;
        let (x, y) = ((m.x * n) as u32, (m.y * n) as u32);
        if let Tile::Data(_) = tile(src, prefs, z, x, y)? {
            return Ok(Some(z));
        }
        z -= 1;
    }
    Ok(None)
}

/// A place found by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Look a place up by name with OpenStreetMap's Nominatim (at most one request a second,
/// as its usage policy asks).
pub fn search(q: &str) -> Result<Vec<Place>, Error> {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            let wait = Duration::from_millis(1100).saturating_sub(t.elapsed());
            std::thread::sleep(wait);
        }
        *last = Some(Instant::now());
    }
    let lang = crate::i18n::code(crate::i18n::get());
    let url = format!("https://nominatim.openstreetmap.org/search?format=json&limit=8&accept-language={lang}&q={}", encode(q));
    let (status, body, _) = get(&url)?;
    if status != 200 {
        return Err(format!("Nominatim: HTTP {status}"));
    }
    parse_places(&String::from_utf8_lossy(&body))
}

pub fn parse_places(body: &str) -> Result<Vec<Place>, Error> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let num = |x: Option<&serde_json::Value>| x.and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or_else(|| x.as_f64()));
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some(Place { name: p.get("display_name")?.as_str()?.to_string(), lat: num(p.get("lat"))?, lon: num(p.get("lon"))? }))
        .collect())
}

/// Ask OpenStreetMap's Overpass API (one request at a time; an answer is kept a week in
/// the cache, so the same place is not asked for again).
pub fn overpass(q: &str) -> Result<String, Error> {
    super::osm_source::fetch(q, "background/compatibility", &std::sync::atomic::AtomicBool::new(false), Instant::now() + Duration::from_secs(240), &|_| {})
}

/// Percent-encoding for a query parameter.
pub(super) fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding() {
        assert_eq!(encode("Rathaus Spandau, Berlin"), "Rathaus+Spandau%2C+Berlin");
        assert_eq!(encode("Москва"), "%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0");
    }

    #[test]
    fn nominatim_answer() {
        let body = r#"[{"place_id":1,"lat":"52.5354","lon":"13.1996","display_name":"Rathaus Spandau, Berlin"},{"lat":"x"}]"#;
        let p = parse_places(body).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Rathaus Spandau, Berlin");
        assert!((p[0].lat - 52.5354).abs() < 1e-9 && (p[0].lon - 13.1996).abs() < 1e-9);
    }

    fn jpeg(f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(256, 256, |x, y| image::Rgb(f(x, y)));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80).encode_image(&img).unwrap();
        out
    }

    #[test]
    fn placeholders() {
        // a flat grey with a line of dark text across it (Esri's "Map data not yet available")
        let grey = jpeg(|x, y| if (120..136).contains(&y) && (40..216).contains(&x) && (x / 3 + y) % 4 == 0 { [60, 60, 60] } else { [204, 204, 204] });
        assert!(is_placeholder(&grey));
        assert!(is_placeholder(b"not a picture"));
        // the sea: flat, but blue
        assert!(!is_placeholder(&jpeg(|_, _| [20, 60, 120])));
        // a town
        assert!(!is_placeholder(&jpeg(|x, y| [((x * 7) ^ (y * 13)) as u8, (x * y) as u8, (x + y) as u8])));
    }
}
