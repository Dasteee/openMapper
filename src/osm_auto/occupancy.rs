//! Broad-phase collision lookup. Exact footprint tests only examine nearby polygons.

use glam::DVec2;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub lo: DVec2,
    pub hi: DVec2,
}
impl Bounds {
    pub fn of(points: &[DVec2]) -> Option<Self> {
        if points.is_empty() || points.iter().any(|p| !p.is_finite()) {
            return None;
        }
        Some(Self {
            lo: points.iter().copied().fold(DVec2::splat(f64::INFINITY), DVec2::min),
            hi: points.iter().copied().fold(DVec2::splat(f64::NEG_INFINITY), DVec2::max),
        })
    }
    pub fn intersects(self, other: Self) -> bool {
        self.lo.x <= other.hi.x && self.hi.x >= other.lo.x && self.lo.y <= other.hi.y && self.hi.y >= other.lo.y
    }
    fn cells(self) -> Option<([i64; 2], [i64; 2])> {
        let lo = [(self.lo.x / 32.0).floor() as i64, (self.lo.y / 32.0).floor() as i64];
        let hi = [(self.hi.x / 32.0).floor() as i64, (self.hi.y / 32.0).floor() as i64];
        let width = hi[0].checked_sub(lo[0])?.checked_add(1)?;
        let height = hi[1].checked_sub(lo[1])?.checked_add(1)?;
        (width.checked_mul(height)? <= 4096).then_some((lo, hi))
    }
}

#[derive(Default)]
pub struct Occupancy {
    polygons: Vec<Vec<DVec2>>,
    bounds: Vec<Bounds>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    large: Vec<usize>,
}
impl Occupancy {
    pub fn from_polygons(polygons: impl IntoIterator<Item = Vec<DVec2>>) -> Self {
        let mut result = Self::default();
        result.extend(polygons);
        result
    }
    pub fn extend(&mut self, polygons: impl IntoIterator<Item = Vec<DVec2>>) {
        for polygon in polygons {
            self.insert(polygon);
        }
    }
    pub fn insert(&mut self, polygon: Vec<DVec2>) {
        let Some(bounds) = Bounds::of(&polygon) else {
            return;
        };
        let id = self.polygons.len();
        if let Some((lo, hi)) = bounds.cells() {
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    self.cells.entry((x, y)).or_default().push(id);
                }
            }
        } else {
            self.large.push(id);
        }
        self.polygons.push(polygon);
        self.bounds.push(bounds);
    }
    pub fn polygons(&self) -> &[Vec<DVec2>] {
        &self.polygons
    }
    pub fn nearby(&self, polygon: &[DVec2]) -> Vec<&Vec<DVec2>> {
        let Some(bounds) = Bounds::of(polygon) else {
            return Vec::new();
        };
        let mut ids: HashSet<usize> = self.large.iter().copied().collect();
        if let Some((lo, hi)) = bounds.cells() {
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    ids.extend(self.cells.get(&(x, y)).into_iter().flatten().copied());
                }
            }
        } else {
            ids.extend(0..self.polygons.len());
        }
        let mut ids: Vec<_> = ids.into_iter().filter(|id| bounds.intersects(self.bounds[*id])).collect();
        ids.sort_unstable();
        ids.into_iter().map(|id| &self.polygons[id]).collect()
    }
    pub fn conflicts(&self, polygon: &[DVec2], holes: &[Vec<DVec2>]) -> bool {
        self.nearby(polygon)
            .into_iter()
            .any(|other| super::buildings::overlaps(other, polygon) && !holes.iter().any(|hole| other.iter().all(|p| super::buildings::contains(*p, hole))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn square(x: f64, y: f64, w: f64) -> Vec<DVec2> {
        vec![DVec2::new(x, y), DVec2::new(x + w, y), DVec2::new(x + w, y + w), DVec2::new(x, y + w)]
    }
    #[test]
    fn lookup_keeps_distant_roads_out_of_exact_collision_tests() {
        let polygons: Vec<_> = (0..20_000).map(|i| square(i as f64 * 50.0, -5.0, 10.0)).collect();
        let grid = Occupancy::from_polygons(polygons.clone());
        let query = square(2.0, -2.0, 2.0);
        assert_eq!(grid.nearby(&query).len(), 1);
        assert!(grid.conflicts(&query, &[]));
        for x in [-25.0, 0.0, 8.0, 20.0, 49.0, 300.0] {
            let query = square(x, -2.0, 3.0);
            assert_eq!(
                grid.conflicts(&query, &[]),
                polygons.iter().any(|p| super::super::buildings::overlaps(p, &query))
            );
        }
    }
    #[test]
    fn large_polygons_negative_cells_and_courtyards_preserve_results() {
        let mut grid = Occupancy::default();
        grid.insert(square(-100_000.0, -100_000.0, 200_000.0));
        let query = square(-10.0, -10.0, 5.0);
        assert!(grid.conflicts(&query, &[]));
        let grid = Occupancy::from_polygons([square(5.0, 5.0, 2.0)]);
        assert!(!grid.conflicts(&square(0.0, 0.0, 20.0), &[square(4.0, 4.0, 10.0)]));
    }
}
