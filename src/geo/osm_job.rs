//! Roads-first imports. Each successful layer is published independently of failures.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use super::osm_source::{self, Progress};
use crate::{osm, osm_buildings, osm_green, osm_lines, osm_water};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Roads,
    Lines,
    Trees,
    Buildings,
    Water,
}

impl Layer {
    pub const ALL: [Self; 5] = [Self::Roads, Self::Lines, Self::Trees, Self::Buildings, Self::Water];
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Roads => "roads/stops",
            Self::Lines => "bus routes",
            Self::Trees => "trees",
            Self::Buildings => "buildings",
            Self::Water => "water",
        }
    }
    fn query(self, bbox: [f64; 4]) -> String {
        match self {
            Self::Roads => osm::query(bbox),
            Self::Lines => osm_lines::query(bbox),
            Self::Trees => osm_green::query(bbox),
            Self::Buildings => osm_buildings::query(bbox),
            Self::Water => osm_water::query(bbox),
        }
    }
    fn parse(self, body: &str, check: &impl Fn() -> Result<(), String>) -> Result<Data, String> {
        let value = osm_source::parse_json(body, check)?;
        match self {
            Self::Roads => Ok(Data::Roads(osm::parse_value(&value, check)?, osm::parse_stops_value(&value, check)?)),
            Self::Lines => osm_lines::parse_value(&value, check).map(Data::Lines),
            Self::Trees => osm_green::parse_value(&value, check).map(Data::Trees),
            Self::Buildings => osm_buildings::parse_value(&value, check).map(Data::Buildings),
            Self::Water => osm_water::parse_value(&value, check).map(Data::Water),
        }
    }
}

pub enum Data {
    Roads(Vec<osm::Way>, Vec<osm::Stop>),
    Lines(Vec<osm_lines::Route>),
    Trees(osm_green::Green),
    Buildings(Vec<osm_buildings::Building>),
    Water(osm_water::Water),
}

pub enum Msg {
    Progress(Progress),
    Layer(Layer, Result<Data, String>),
    Done(Result<(), String>),
}

pub struct Job {
    pub rx: Receiver<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(bbox: [f64; 4], selected: [bool; 5]) -> Result<Job, String> {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::Builder::new()
        .name("osm-import".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(selected, &tx, &flag, Instant::now() + Duration::from_secs(360), |layer, deadline| {
                    let body = osm_source::fetch(&layer.query(bbox), layer.name(), &flag, deadline, &|p| {
                        let _ = tx.send(Msg::Progress(p));
                    })?;
                    osm_source::check(&flag, deadline)?;
                    let _ = tx.send(Msg::Progress(Progress {
                        dataset: layer.name().into(),
                        query: super::fetch::fnv(layer.query(bbox).as_bytes()),
                        phase: "parsing",
                        endpoint: String::new(),
                        attempt: 0,
                        status: None,
                        queue_ms: 0,
                        elapsed_ms: 0,
                        bytes: body.len(),
                        detail: String::new(),
                    }));
                    let parsing = Instant::now();
                    let parsed = layer.parse(&body, &|| osm_source::check(&flag, deadline));
                    log::info!("OSM dataset={} phase=parsed elapsed_ms={} success={}", layer.name(), parsing.elapsed().as_millis(), parsed.is_ok());
                    osm_source::check(&flag, deadline)?;
                    parsed
                })
            }))
            .unwrap_or_else(|_| Err("OSM import worker panicked (see log)".into()));
            if let Err(e) = &result {
                log::warn!("OSM job: {e}");
            }
            let _ = tx.send(Msg::Done(result));
        })
        .map_err(|e| format!("Cannot start OSM import: {e}"))?;
    Ok(Job {
        rx,
        cancel,
        started: Instant::now(),
    })
}

/// Saved Overpass JSON is parsed through the same adapters as network responses.
pub fn local(path:std::path::PathBuf)->Result<Job,String>{
    let (tx,rx)=channel();let cancel=Arc::new(AtomicBool::new(false));let flag=cancel.clone();
    std::thread::Builder::new().name("osm-local".into()).spawn(move||{
        let deadline=Instant::now()+Duration::from_secs(120);
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            use std::io::Read;
            let mut body=String::new();
            std::fs::File::open(&path).map_err(|e|e.to_string())?.take((64<<20)+1).read_to_string(&mut body).map_err(|e|e.to_string())?;
            if body.len()>64<<20{return Err("OSM JSON exceeds 64 MiB".into());}
            let value=osm_source::parse_json(&body,&||osm_source::check(&flag,deadline))?;
            if value.get("remark").is_some()||!value.get("elements").is_some_and(|v|v.is_array()){return Err("Incomplete OSM JSON".into());}
            run([true;5],&tx,&flag,deadline,|layer,_|layer.parse(&body,&||osm_source::check(&flag,deadline)))
        })).unwrap_or_else(|_|Err("OSM local worker panicked".into()));
        let _=tx.send(Msg::Done(result));
    }).map_err(|e|e.to_string())?;
    Ok(Job{rx,cancel,started:Instant::now()})
}

fn run(
    selected: [bool; 5],
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    deadline: Instant,
    mut acquire: impl FnMut(Layer, Instant) -> Result<Data, String>,
) -> Result<(), String> {
    for layer in Layer::ALL {
        if !selected[layer.index()] {
            continue;
        }
        osm_source::check(cancel, deadline)?;
        let result = acquire(layer, deadline);
        osm_source::check(cancel, deadline)?;
        if let Err(e) = &result {
            log::warn!("OSM {}: {e}", layer.name());
        }
        tx.send(Msg::Layer(layer, result)).map_err(|_| "OSM import window closed".to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_mixed_response_keeps_layer_types_separate() {
        let path=std::env::temp_dir().join(format!("osm-mixed-{}.json",std::process::id()));
        std::fs::write(&path,r#"{"elements":[{"type":"node","id":1,"lat":1,"lon":2,"tags":{"highway":"bus_stop"}},{"type":"node","id":2,"lat":3,"lon":4,"tags":{"natural":"tree"}},{"type":"way","id":3,"tags":{"building":"house"},"geometry":[{"lat":0,"lon":0},{"lat":0,"lon":1},{"lat":1,"lon":1},{"lat":0,"lon":0}]},{"type":"way","id":4,"tags":{"natural":"water"},"geometry":[{"lat":2,"lon":2},{"lat":2,"lon":3},{"lat":3,"lon":3},{"lat":2,"lon":2}]}]}"#).unwrap();
        let job=local(path.clone()).unwrap();let mut layers=0;
        loop {match job.rx.recv_timeout(Duration::from_secs(3)).unwrap(){
            Msg::Layer(_,Ok(Data::Roads(roads,stops)))=>{assert!(roads.is_empty());assert_eq!(stops.len(),1);layers+=1;},
            Msg::Layer(_,Ok(Data::Trees(green)))=>{assert_eq!(green.trees.len(),1);assert!(green.woods.is_empty());layers+=1;},
            Msg::Layer(_,Ok(Data::Buildings(buildings)))=>{assert_eq!(buildings.len(),1);layers+=1;},
            Msg::Layer(_,Ok(Data::Water(water)))=>{assert_eq!(water.areas.len(),1);layers+=1;},
            Msg::Layer(_,Ok(Data::Lines(lines)))=>{assert!(lines.is_empty());layers+=1;},
            Msg::Done(result)=>{result.unwrap();break;},
            _=>panic!("unexpected local import error"),
        }}
        assert_eq!(layers,5);std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn roads_first_disabled_layers_skipped_and_success_survives_failure() {
        let (tx, rx) = channel();
        let mut calls = Vec::new();
        run(
            [true, true, false, false, true],
            &tx,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(1),
            |layer, _| {
                calls.push(layer);
                match layer {
                    Layer::Roads => Ok(Data::Roads(vec![], vec![])),
                    Layer::Lines => Err("busy".into()),
                    Layer::Water => Ok(Data::Water(Default::default())),
                    _ => panic!("disabled layer requested"),
                }
            },
        )
        .unwrap();
        assert_eq!(calls, [Layer::Roads, Layer::Lines, Layer::Water]);
        assert!(matches!(rx.try_recv().unwrap(), Msg::Layer(Layer::Roads, Ok(_))));
        assert!(matches!(rx.try_recv().unwrap(), Msg::Layer(Layer::Lines, Err(_))));
        assert!(matches!(rx.try_recv().unwrap(), Msg::Layer(Layer::Water, Ok(_))));
    }

    #[test]
    fn cancellation_does_not_publish_late_data() {
        let (tx, rx) = channel();
        let cancel = AtomicBool::new(false);
        assert!(
            run([true; 5], &tx, &cancel, Instant::now() + Duration::from_secs(1), |_, _| {
                cancel.store(true, Ordering::Relaxed);
                Ok(Data::Roads(vec![], vec![]))
            })
            .is_err()
        );
        assert!(rx.try_recv().is_err());
    }
}
