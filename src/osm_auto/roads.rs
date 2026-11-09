use super::index::RoadAsset;
use crate::osm::{Class, Road};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub forward: u8,
    pub backward: u8,
    pub shared: bool,
    pub width: f64,
    pub left_walk: f64,
    pub right_walk: f64,
    pub surface: String,
    pub inferred: bool,
}

pub fn target(road: &Road) -> Result<Target, String> {
    let s = &road.source;
    if !s.restrictions.is_empty() {
        return Err(format!(
            "Turn restrictions {:?} require junction review",
            s.restrictions.iter().map(|r| r.id).collect::<Vec<_>>()
        ));
    }
    if s.tag("bridge") != "" && s.tag("bridge") != "no" || s.tag("tunnel") != "" && s.tag("tunnel") != "no" || s.tag("layer").parse::<i32>().unwrap_or(0) != 0 {
        return Err("Grade-separated road requires elevation and junction review".into());
    }
    if ["access", "vehicle", "motor_vehicle"].iter().any(|key| matches!(s.tag(key), "no" | "private"))
        && !matches!(s.tag("bus"), "yes" | "designated" | "permissive")
    {
        return Err("Motor vehicle access is restricted".into());
    }
    let count = |text: &str| text.parse::<u8>().ok().filter(|n| *n <= 12);
    let total = count(s.tag("lanes")).filter(|n| *n > 0);
    let fwd = count(s.directional("lanes", true));
    let back = count(s.directional("lanes", false));
    if (!s.tag("lanes").is_empty() && total.is_none()) || count(s.tag("lanes:both_ways")).is_some_and(|n| n > 0) {
        return Err("Ambiguous lane counts or a centre turning lane require review".into());
    }
    let counts_inferred = total.is_none() && fwd.zip(back).is_none();
    let prior = match road.class {
        Class::Main if matches!(s.tag("highway"), "motorway" | "trunk") => 4,
        _ => 2,
    };
    let total = total
        .or_else(|| fwd.zip(back).map(|(a, b)| a + b))
        .unwrap_or(if road.oneway { if road.class == Class::Main { 2 } else { 1 } } else { prior });
    let mut shared = !road.oneway && total == 1;
    let (mut forward, mut backward) = if road.oneway {
        (total, 0)
    } else if shared {
        (1, 0)
    } else {
        let f = fwd.unwrap_or_else(|| back.map(|b| total.saturating_sub(b)).unwrap_or(total.div_ceil(2)));
        (f, back.unwrap_or(total.saturating_sub(f)))
    };
    if forward == 0
        || (!road.oneway && !shared && backward == 0)
        || forward + backward > 12
        || (!shared && forward + backward != total)
        || (road.oneway && back.is_some_and(|n| n > 0))
    {
        return Err("Inconsistent directional lane counts".into());
    }
    let measured = crate::osm_buildings::measure(s.tag("width")).or_else(|| crate::osm_buildings::measure(s.tag("est_width")));
    let lane_widths: Option<Vec<f64>> = if s.tag("width:lanes").is_empty() {
        None
    } else {
        s.tag("width:lanes").split('|').map(crate::osm_buildings::measure).collect()
    };
    let prior = if road.class == Class::Main { 3.5 } else { 3.0 };
    let width = measured.or_else(|| lane_widths.map(|w| w.iter().sum())).unwrap_or(total as f64 * prior);
    if !width.is_finite() || !(2.0..=50.0).contains(&width) {
        return Err("Unsupported or invalid road width".into());
    }
    if !road.oneway && counts_inferred && width < 4.75 {
        shared = true;
        forward = 1;
        backward = 0;
    }
    if !shared && width / ((forward + backward) as f64) < 2.25 {
        return Err("The tagged width cannot accommodate its lane counts".into());
    }
    let sidewalk = |left: bool| {
        let left = left != s.reversed;
        let side = if left { "left" } else { "right" };
        let specific = s.tags.get(&format!("sidewalk:{side}")).map(String::as_str).unwrap_or("");
        let present = if specific.is_empty() {
            match s.tag("sidewalk") {
                "both" | "yes" => true,
                "left" => left,
                "right" => !left,
                _ => false,
            }
        } else {
            matches!(specific, "yes" | "both" | "designated")
        };
        if present {
            s.tags
                .get(&format!("sidewalk:{side}:width"))
                .and_then(|s| crate::osm_buildings::measure(s))
                .unwrap_or(1.8)
                .clamp(0.5, 8.0)
        } else {
            0.0
        }
    };
    Ok(Target {
        forward,
        backward,
        shared,
        width,
        left_walk: sidewalk(true),
        right_walk: sidewalk(false),
        surface: s.tag("surface").into(),
        inferred: counts_inferred || measured.is_none(),
    })
}

pub fn rank(target: &Target, candidates: &[RoadAsset], rail: bool) -> Vec<(f64, RoadAsset)> {
    let mut result = Vec::new();
    for a in candidates {
        if rail {
            if a.rail == 0 || a.forward + a.backward + a.shared > 0 {
                continue;
            }
            result.push((0.0, a.clone()));
            continue;
        }
        if a.rail > 0
            || if target.shared {
                a.shared != 1 || a.forward + a.backward > 0
            } else {
                a.shared > 0 || a.forward != target.forward || a.backward != target.backward
            }
        {
            continue;
        }
        let width = (a.carriageway - target.width).abs() / target.width;
        if width > 0.3 {
            continue;
        }
        let sidewalk = (a.left_walk != (target.left_walk > 0.0)) as u8 as f64 + (a.right_walk != (target.right_walk > 0.0)) as u8 as f64;
        let surface = if target.surface.is_empty() || a.surface.is_empty() {
            0.15
        } else if target.surface == a.surface {
            0.0
        } else {
            0.75
        };
        result.push((2.0 * width + sidewalk + surface, a.clone()));
    }
    result.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.rel.cmp(&b.1.rel)));
    result.truncate(4);
    result
}

pub fn spec(target: &Target) -> crate::road_maker::RoadSpec {
    let key = crate::geo::fetch::fnv(&serde_json::to_vec(target).unwrap_or_default());
    let mut spec = crate::road_maker::RoadSpec {
        name: format!("osm_{key:016x}"),
        lanes_fwd: target.forward,
        lanes_back: target.backward,
        lane_width: (target.width / (target.forward + target.backward) as f64) as f32,
        footway: target.left_walk as f32,
        footway_right: Some(target.right_walk as f32),
        kerb: if target.left_walk + target.right_walk > 0.0 { 0.12 } else { 0.0 },
        ..Default::default()
    };
    spec.shared_lane = target.shared;
    spec.marked = !target.shared && target.width >= 5.0 && matches!(target.surface.as_str(), "" | "asphalt" | "paved" | "concrete");
    if spec.marked {
        spec.markings = spec.standard_markings();
        for marking in &mut spec.markings {
            marking.color = crate::road_maker::WHITE;
        }
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;
    fn road(tags: &[(&str, &str)], oneway: bool) -> Road {
        Road {
            class: Class::Street,
            oneway,
            points: vec![],
            source: crate::osm::RoadSource {
                tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                ..Default::default()
            },
            asset: None,
        }
    }
    #[test]
    fn measured_width_directional_lanes_and_asymmetric_sidewalk() {
        let r = road(&[("lanes", "3"), ("lanes:forward", "2"), ("width", "9 m"), ("sidewalk", "left")], false);
        let t = target(&r).unwrap();
        assert_eq!((t.forward, t.backward, t.width, t.left_walk, t.right_walk), (2, 1, 9.0, 1.8, 0.0));
        let mut a = RoadAsset {
            rel: "exact".into(),
            forward: 2,
            backward: 1,
            shared: 0,
            rail: 0,
            left_walk: true,
            right_walk: false,
            carriageway: 9.0,
            bounds: [-6.0, 4.5],
            surface: "asphalt".into(),
        };
        let exact = a.clone();
        a.rel = "wrong direction".into();
        a.forward = 1;
        a.backward = 2;
        let matches = rank(&t, &[a, exact], false);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].1.rel, "exact");
    }
    #[test]
    fn narrow_two_way_is_a_shared_lane_and_grade_separation_is_explicit() {
        let t = target(&road(&[("lanes", "1"), ("width", "3.5")], false)).unwrap();
        assert!(t.shared && spec(&t).shared_lane);
        assert!(target(&road(&[("bridge", "yes")], false)).is_err());
        assert!(target(&road(&[("lanes", "2"), ("lanes:forward", "2")], false)).is_err());
    }
}
