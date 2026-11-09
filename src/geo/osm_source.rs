//! Cancellable Overpass acquisition, shared by the importer and the background map.
//! Cache entries are complete JSON responses, including valid empty datasets.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;

use super::fetch::{USER_AGENT, cache_root, fnv};

const MAX_BODY: usize = 64 << 20;
const POLL: Duration = Duration::from_millis(50);
const ATTEMPT: Duration = Duration::from_secs(105);

/// One observable transition. Bounded server explanations are included in `detail`.
#[derive(Clone, Debug)]
pub struct Progress {
    pub dataset: String,
    pub query: u64,
    pub phase: &'static str,
    pub endpoint: String,
    pub attempt: usize,
    pub status: Option<u16>,
    pub queue_ms: u128,
    pub elapsed_ms: u128,
    pub bytes: usize,
    pub detail: String,
}

struct Source {
    client: reqwest::Client,
    endpoints: Vec<String>,
    cache: PathBuf,
    gate: Arc<Semaphore>,
    attempt_budget: Duration,
}

fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

fn source() -> Result<&'static Source, String> {
    static SOURCE: OnceLock<Result<Source, String>> = OnceLock::new();
    SOURCE
        .get_or_init(|| {
            Ok(Source {
                client: reqwest::Client::builder()
                    .user_agent(USER_AGENT)
                    .connect_timeout(Duration::from_secs(10))
                    .read_timeout(Duration::from_secs(95))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| e.to_string())?,
                endpoints: vec![
                    "https://overpass-api.de/api/interpreter".into(),
                    "https://maps.mail.ru/osm/tools/overpass/api/interpreter".into(),
                ],
                cache: cache_root().parent().unwrap_or(Path::new(".")).join("osm"),
                gate: Arc::new(Semaphore::new(1)),
                attempt_budget: ATTEMPT,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub fn check(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("OSM import cancelled".into())
    } else if Instant::now() >= deadline {
        Err("OSM import deadline exceeded".into())
    } else {
        Ok(())
    }
}

/// Race actual network/queue/backoff work against cancellation; dropping the future also
/// drops an in-flight response. No detached HTTP worker holds the queue after Cancel.
async fn interrupt<T>(future: impl std::future::Future<Output = Result<T, String>>, cancel: &AtomicBool, deadline: Instant) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        check(cancel, deadline)?;
        tokio::select! {
            result = &mut future => { check(cancel, deadline)?; return result; }
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}

fn excerpt(text: &str) -> String {
    text.chars().take(400).map(|c| if c.is_control() { ' ' } else { c }).collect()
}

#[cfg(test)]
fn validate(text: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("Invalid Overpass JSON: {e}"))?;
    validate_value(&v)
}

fn validate_value(v: &serde_json::Value) -> Result<(), String> {
    if let Some(remark) = v.get("remark") {
        return Err(format!("Incomplete Overpass response: {}", excerpt(&remark.to_string())));
    }
    if !v.get("elements").is_some_and(|e| e.is_array()) {
        return Err("Incomplete Overpass response: missing elements array".into());
    }
    Ok(())
}

/// serde's reader checks cancellation every few KiB, including within a large element.
/// This path is used by importer workers; existing synchronous parser callers keep the
/// faster slice deserializer through their compatibility entry points.
pub fn parse_json(text: &str, check: &impl Fn() -> Result<(), String>) -> Result<serde_json::Value, String> {
    struct Checked<'a, F> {
        input: &'a [u8],
        check: &'a F,
        until_check: usize,
    }
    impl<F: Fn() -> Result<(), String>> std::io::Read for Checked<'_, F> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.until_check == 0 {
                (self.check)().map_err(std::io::Error::other)?;
                self.until_check = 4096;
            }
            let n = out.len().min(self.input.len()).min(self.until_check);
            out[..n].copy_from_slice(&self.input[..n]);
            self.input = &self.input[n..];
            self.until_check -= n;
            Ok(n)
        }
    }
    check()?;
    let value = serde_json::from_reader(Checked {
        input: text.as_bytes(),
        check,
        until_check: 0,
    })
    .map_err(|e| e.to_string())?;
    check()?;
    Ok(value)
}

#[cfg(test)]
fn cached(path: &Path) -> Option<String> {
    cached_checked(path, &|| Ok(()))
}

fn cached_checked(path: &Path, check: &impl Fn() -> Result<(), String>) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_BODY as u64 || meta.modified().ok()?.elapsed().ok()?.as_secs() >= 7 * 86_400 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let value = parse_json(&text, check).ok()?;
    validate_value(&value).ok()?;
    Some(text)
}

fn keep(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    // Network requests for this cache are serialized. Readers see the previous complete
    // entry or the new complete entry, never an interrupted write.
    let tmp = path.with_extension(format!("{}.part", std::process::id()));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn fetch(q: &str, dataset: &str, cancel: &AtomicBool, deadline: Instant, progress: &impl Fn(Progress)) -> Result<String, String> {
    runtime()?.block_on(fetch_from(source()?, q, dataset, cancel, deadline, progress))
}

async fn fetch_from(source: &Source, q: &str, dataset: &str, cancel: &AtomicBool, deadline: Instant, progress: &impl Fn(Progress)) -> Result<String, String> {
    let started = Instant::now();
    let path = source.cache.join(format!("{:016x}.json", fnv(q.as_bytes())));
    let mut event = Progress {
        dataset: dataset.into(),
        query: fnv(q.as_bytes()),
        phase: "cache",
        endpoint: String::new(),
        attempt: 0,
        status: None,
        queue_ms: 0,
        elapsed_ms: 0,
        bytes: 0,
        detail: String::new(),
    };
    let emit = |event: &Progress| {
        log::info!(
            "OSM dataset={} query={:016x} phase={} endpoint={} attempt={} status={:?} queue_ms={} elapsed_ms={} bytes={} detail={}",
            event.dataset,
            event.query,
            event.phase,
            event.endpoint,
            event.attempt,
            event.status,
            event.queue_ms,
            event.elapsed_ms,
            event.bytes,
            event.detail
        );
        progress(event.clone());
    };
    check(cancel, deadline)?;
    emit(&event);
    if let Some(text) = cached_checked(&path, &|| check(cancel, deadline)) {
        check(cancel, deadline)?;
        event.phase = "cached";
        event.bytes = text.len();
        event.elapsed_ms = started.elapsed().as_millis();
        emit(&event);
        return Ok(text);
    }
    event.phase = "queued";
    emit(&event);
    let queued = Instant::now();
    let _permit = interrupt(async { source.gate.acquire().await.map_err(|e| e.to_string()) }, cancel, deadline).await?;
    event.queue_ms = queued.elapsed().as_millis();
    // Another request for the same query may have filled the cache while we queued.
    if let Some(text) = cached_checked(&path, &|| check(cancel, deadline)) {
        check(cancel, deadline)?;
        event.phase = "cached";
        event.bytes = text.len();
        emit(&event);
        return Ok(text);
    }
    let mut failures = Vec::new();
    for (i, endpoint) in source.endpoints.iter().enumerate() {
        check(cancel, deadline)?;
        event.endpoint = endpoint.clone();
        event.attempt = i + 1;
        event.status = None;
        event.bytes = 0;
        event.detail.clear();
        event.phase = "requesting";
        event.elapsed_ms = 0;
        emit(&event);
        let attempt = Instant::now();
        let mut retry_after = None;
        let attempt_deadline = deadline.min(attempt + source.attempt_budget);
        let got = interrupt(
            async {
                let mut response = source
                    .client
                    .post(endpoint)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(format!("data={}", super::fetch::encode(q)))
                    .send()
                    .await
                    .map_err(|e| format!("Connection/request failure: {e:?}"))?;
                event.status = Some(response.status().as_u16());
                retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_owned);
                event.detail = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").into();
                event.phase = "reading";
                emit(&event);
                if response.content_length().is_some_and(|n| n > MAX_BODY as u64) {
                    return Err("Overpass response exceeds 64 MiB".into());
                }
                let mut body = Vec::new();
                let mut updated = Instant::now();
                while let Some(chunk) = response.chunk().await.map_err(|e| format!("Response read failure: {e:?}"))? {
                    if body.len() + chunk.len() > MAX_BODY {
                        return Err("Overpass response exceeds 64 MiB".into());
                    }
                    body.extend_from_slice(&chunk);
                    event.bytes = body.len();
                    check(cancel, deadline)?;
                    if updated.elapsed() >= Duration::from_millis(200) {
                        event.elapsed_ms = attempt.elapsed().as_millis();
                        emit(&event);
                        updated = Instant::now();
                    }
                }
                String::from_utf8(body).map_err(|e| format!("Invalid Overpass UTF-8: {e}"))
            },
            cancel,
            attempt_deadline,
        )
        .await
        .map_err(|e| {
            if !cancel.load(Ordering::Relaxed) && Instant::now() >= attempt_deadline && attempt_deadline < deadline {
                format!("Overpass attempt timed out after {} ms", attempt.elapsed().as_millis())
            } else {
                e
            }
        });
        event.elapsed_ms = attempt.elapsed().as_millis();
        check(cancel, deadline)?;
        let result = got.and_then(|text| {
            if event.status != Some(200) {
                return Err(format!("HTTP {}: {}", event.status.unwrap_or(0), excerpt(&text)));
            }
            event.phase = "validating";
            emit(&event);
            let value = parse_json(&text, &|| check(cancel, deadline)).map_err(|e| format!("Invalid Overpass JSON: {e}"))?;
            validate_value(&value)?;
            Ok(text)
        });
        match result {
            Ok(text) => {
                check(cancel, deadline)?;
                if let Err(e) = keep(&path, &text) {
                    log::warn!("OSM cache write {}: {e}", path.display());
                }
                event.phase = "ready";
                event.detail.clear();
                emit(&event);
                return Ok(text);
            }
            Err(error) => {
                event.phase = "failed";
                event.detail = error.clone();
                emit(&event);
                failures.push(format!("{dataset}: {endpoint}: {error}"));
                // An unchanged query rejected as invalid is not retried elsewhere.
                if event.status.is_some_and(|s| !matches!(s, 200 | 429 | 502 | 503 | 504)) {
                    break;
                }
                if i + 1 < source.endpoints.len() {
                    // A long or date-form Retry-After is surfaced rather than ignored.
                    let seconds = match retry_after {
                        Some(ref h) => match h.trim().parse::<u64>() {
                            Ok(s) if s <= 60 => s,
                            _ => {
                                failures.push(format!("Retry-After: {}", excerpt(h)));
                                break;
                            }
                        },
                        None => 1,
                    };
                    event.phase = "backoff";
                    emit(&event);
                    interrupt(
                        async {
                            tokio::time::sleep(Duration::from_secs(seconds)).await;
                            Ok(())
                        },
                        cancel,
                        deadline,
                    )
                    .await?;
                }
            }
        }
    }
    Err(failures.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};

    /// A single bounded local request. Reads the POST body before returning a fixture.
    fn server(handle: impl FnOnce(std::net::TcpStream) + Send + 'static) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let start = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && start.elapsed() < Duration::from_secs(5) => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("local fixture accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut reader = std::io::BufReader::new(&mut stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("POST / HTTP/1.1"), "{line}");
            let mut length = 0;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            assert!(body.starts_with(b"data="));
            handle(stream);
        });
        (endpoint, thread)
    }

    fn reply(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<()>) {
        server(move |mut stream| {
            write!(
                stream,
                "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        })
    }

    fn fixture_source(endpoints: Vec<String>, cache: PathBuf) -> Source {
        Source {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            endpoints,
            cache,
            gate: Arc::new(Semaphore::new(1)),
            attempt_budget: Duration::from_secs(2),
        }
    }

    #[test]
    fn fallback_records_both_attempts_and_warm_cache_makes_no_request() {
        let dir = temp();
        let (busy, first) = reply(503, "temporarily unavailable");
        let (ok, second) = reply(200, r#"{"elements":[]}"#);
        let mut source = fixture_source(vec![busy, ok], dir.clone());
        let events = std::sync::Mutex::new(Vec::new());
        let cancel = AtomicBool::new(false);
        let body = runtime()
            .unwrap()
            .block_on(fetch_from(
                &source,
                "fixture",
                "roads",
                &cancel,
                Instant::now() + Duration::from_secs(5),
                &|p| events.lock().unwrap().push(p),
            ))
            .unwrap();
        first.join().unwrap();
        second.join().unwrap();
        let events = events.into_inner().unwrap();
        assert!(
            events
                .iter()
                .any(|p| p.phase == "failed" && p.status == Some(503) && p.detail.contains("temporarily unavailable"))
        );
        assert!(events.iter().any(|p| p.phase == "ready" && p.status == Some(200) && p.attempt == 2));
        source.endpoints = vec!["http://127.0.0.1:1".into()];
        assert_eq!(
            runtime()
                .unwrap()
                .block_on(fetch_from(
                    &source,
                    "fixture",
                    "roads",
                    &cancel,
                    Instant::now() + Duration::from_secs(1),
                    &|_| {}
                ))
                .unwrap(),
            body
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn incomplete_malformed_and_query_errors_are_not_cached() {
        for (status, body, error) in [
            (200, r#"{"elements":[],"remark":"runtime error"}"#, "Incomplete"),
            (200, "{}", "missing elements"),
            (200, "{", "Invalid"),
            (400, "parse error at line 1", "HTTP 400"),
        ] {
            let dir = temp();
            let (endpoint, thread) = reply(status, body);
            let source = fixture_source(vec![endpoint], dir.clone());
            let got = runtime().unwrap().block_on(fetch_from(
                &source,
                "fixture",
                "roads",
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(3),
                &|_| {},
            ));
            assert!(got.unwrap_err().contains(error));
            thread.join().unwrap();
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn stalled_headers_and_body_are_cancelled_and_release_queue() {
        for headers in [false, true] {
            let dir = temp();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (endpoint, thread) = server(move |mut stream| {
                if headers {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{").unwrap();
                }
                std::thread::sleep(Duration::from_millis(100));
                flag.store(true, Ordering::Relaxed);
                // Dropping the request closes the actual socket, rather than hiding a
                // blocking worker that still owns the network permit.
                let mut byte = [0];
                match stream.read(&mut byte) {
                    Ok(0) => {}
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {}
                    other => panic!("request did not close: {other:?}"),
                }
            });
            let source = fixture_source(vec![endpoint], dir.clone());
            let start = Instant::now();
            let got = runtime()
                .unwrap()
                .block_on(fetch_from(&source, "fixture", "roads", &cancel, start + Duration::from_secs(3), &|_| {}));
            assert!(got.unwrap_err().contains("cancelled"));
            assert!(start.elapsed() < Duration::from_secs(1));
            assert_eq!(source.gate.available_permits(), 1);
            thread.join().unwrap();
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn completeness() {
        assert!(validate(r#"{"elements":[]}"#).is_ok());
        assert!(validate(r#"{"elements":[{"type":"node","id":1}]}"#).is_ok());
        for text in [
            r#"{"elements":[],"remark":"runtime error"}"#,
            "{}",
            r#"{"elements":{}}"#,
            "{",
            "<html>busy</html>",
        ] {
            assert!(validate(text).is_err(), "{text}");
        }
    }

    fn temp() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("openmapper-osm-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn empty_and_corrupt_cache() {
        let dir = temp();
        let path = dir.join("empty.json");
        keep(&path, r#"{"elements":[]}"#).unwrap();
        assert_eq!(cached(&path).unwrap(), r#"{"elements":[]}"#);
        keep(&path, r#"{"elements":[{"id":2}]}"#).unwrap();
        assert!(cached(&path).unwrap().contains("2"));
        std::fs::write(path.with_extension("part"), "{interrupted").unwrap();
        assert!(cached(&path).is_some());
        std::fs::remove_file(path.with_extension("part")).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(8 * 86_400);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        assert!(cached(&path).is_none());
        std::fs::write(&path, "{interrupted").unwrap();
        assert!(cached(&path).is_none());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queue_cancellation_and_cache_bypass() {
        let dir = temp();
        let source = Source {
            client: reqwest::Client::new(),
            endpoints: vec![],
            cache: dir.clone(),
            gate: Arc::new(Semaphore::new(0)),
            attempt_budget: ATTEMPT,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        assert!(
            runtime()
                .unwrap()
                .block_on(fetch_from(&source, "missing", "roads", &cancel, start + Duration::from_secs(3), &|_| {}))
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        worker.join().unwrap();
        cancel.store(false, Ordering::Relaxed);
        keep(&dir.join(format!("{:016x}.json", fnv(b"empty"))), r#"{"elements":[]}"#).unwrap();
        assert!(
            runtime()
                .unwrap()
                .block_on(fetch_from(&source, "empty", "roads", &cancel, Instant::now() + Duration::from_secs(1), &|_| {}))
                .is_ok()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancellation_inside_json_and_overall_deadline() {
        let input = format!(r#"{{"elements":[{}]}}"#, "0,".repeat(20_000) + "0");
        let checks = std::cell::Cell::new(0);
        let result = parse_json(&input, &|| {
            checks.set(checks.get() + 1);
            if checks.get() >= 4 { Err("cancelled during JSON".into()) } else { Ok(()) }
        });
        assert!(result.unwrap_err().contains("cancelled during JSON"));
        let dir = temp();
        let source = fixture_source(vec!["http://127.0.0.1:1".into()], dir.clone());
        let result = runtime()
            .unwrap()
            .block_on(fetch_from(&source, "fixture", "roads", &AtomicBool::new(false), Instant::now(), &|_| {}));
        assert!(result.unwrap_err().contains("deadline"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backoff_cancellation_honors_retry_after_without_another_request() {
        let dir = temp();
        let (endpoint, thread) = server(|mut stream| {
            stream
                .write_all(b"HTTP/1.1 429 Busy\r\nRetry-After: 60\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy")
                .unwrap();
        });
        let source = fixture_source(vec![endpoint, "http://127.0.0.1:1".into()], dir.clone());
        let cancel = AtomicBool::new(false);
        let result = runtime().unwrap().block_on(fetch_from(
            &source,
            "fixture",
            "roads",
            &cancel,
            Instant::now() + Duration::from_secs(3),
            &|p| {
                if p.phase == "backoff" {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        ));
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(source.gate.available_permits(), 1);
        thread.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_and_truncated_bodies_rejected() {
        for length in [MAX_BODY + 1, 100] {
            let dir = temp();
            let (endpoint, thread) = server(move |mut stream| {
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{{").unwrap();
            });
            let source = fixture_source(vec![endpoint], dir.clone());
            let result = runtime().unwrap().block_on(fetch_from(
                &source,
                "fixture",
                "roads",
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(3),
                &|_| {},
            ));
            let error = result.unwrap_err();
            assert!(error.contains(if length > MAX_BODY { "exceeds" } else { "read failure" }), "{error}");
            thread.join().unwrap();
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
