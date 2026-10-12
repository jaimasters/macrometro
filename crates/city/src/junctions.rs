//! R31 road geometry by class: minimum curve radius (fast roads never kink), collinear-node merging, and what every
//! junction becomes: a roundabout sized by its widest road, or a grade-separated overpass where a freeway passes.
use crate::lanes::LaneClass;
use crate::roadgen::RoadGraph;
use bevy::math::Vec2;

/// Smallest curve radius a road of this class may take (world units). Streets may turn sharply.
pub fn min_radius(c: LaneClass) -> f32 {
    [0.0, 2.0, 6.0, 10.0, 18.0, 22.0, 26.0][c as usize]
}

pub fn is_freeway(c: LaneClass) -> bool {
    c.lanes() >= 6
}

/// Radius of the tightest arc that rounds the corner p-n-q using at most half of each leg (infinite when straight).
pub fn kink_radius(p: Vec2, n: Vec2, q: Vec2) -> f32 {
    let (u, v) = ((n - p).normalize_or_zero(), (q - n).normalize_or_zero());
    let turn = u.angle_to(v).abs();
    if turn < 1e-4 {
        return f32::INFINITY;
    }
    0.5 * p.distance(n).min(n.distance(q)) / (turn / 2.0).tan()
}

/// Turns gentler than this at a through node are merged onto the straight line.
const COLLINEAR: f32 = 0.14; // ~8 degrees

/// Re-align every through node (exactly two roads) whose corner is tighter than the faster road's minimum radius, or
/// nearly collinear: it slides toward the chord of its neighbours (only to where `ok`, e.g. land). Edge lengths are
/// refreshed. Returns the nodes moved. Edge/node indices never change, so saves keyed by edge stay valid.
pub fn smooth(g: &mut RoadGraph, class: impl Fn(usize) -> LaneClass, ok: impl Fn(Vec2) -> bool) -> Vec<u32> {
    let inc = g.incident();
    let mut moved = vec![];
    for _ in 0..12 {
        let mut any = false;
        for (n, es) in inc.iter().enumerate() {
            let [e1, e2] = es[..] else { continue };
            let other = |e: usize| if g.edges[e].a as usize == n { g.edges[e].b } else { g.edges[e].a } as usize;
            let (p, q, x) = (g.nodes[other(e1)], g.nodes[other(e2)], g.nodes[n]);
            let r = kink_radius(p, x, q);
            let turn = (x - p).angle_to(q - x).abs();
            let need = min_radius(class(e1)).max(min_radius(class(e2)));
            if r >= need && (turn >= COLLINEAR || turn < 1e-3) {
                continue;
            }
            // collinear: project onto the chord; too tight: halve the way to the chord's midpoint
            let to = if turn < COLLINEAR { let d = (q - p).normalize_or_zero(); p + d * (x - p).dot(d) } else { x.lerp((p + q) / 2.0, 0.5) };
            if to.distance(x) < 1e-3 || !ok(to) {
                continue;
            }
            g.nodes[n] = to;
            if !moved.contains(&(n as u32)) {
                moved.push(n as u32);
            }
            any = true;
        }
        if !any {
            break;
        }
    }
    for e in &mut g.edges {
        e.len = g.nodes[e.a as usize].distance(g.nodes[e.b as usize]);
    }
    moved
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Junction {
    /// A through node or dead end: no junction to draw.
    None,
    /// At-grade roundabout of this ring radius; entering cars give way to the ring.
    Roundabout(f32),
    /// A freeway passes over (or carries ramps to) the other roads: nothing meets at grade, freeway traffic never stops.
    Overpass,
}

/// What node `n` with these incident road classes becomes.
pub fn junction(classes: &[LaneClass]) -> Junction {
    if classes.iter().any(|&c| is_freeway(c)) && classes.len() >= 3 {
        return Junction::Overpass;
    }
    if classes.len() < 3 {
        return Junction::None;
    }
    let w = classes.iter().map(|c| c.width()).fold(0.0, f32::max);
    Junction::Roundabout(2.6 + w * 0.5)
}

/// A roundabout: cars circulate at radius `r` around `c`; `legs` are the approach angles (radians, ascending).
#[derive(Debug, Clone)]
pub struct Ring {
    pub c: Vec2,
    pub r: f32,
    pub legs: Vec<f32>,
}

impl Ring {
    /// Ring for node `n` of `g`, or None if it is not an at-grade junction. The ring never reaches past 40% of its
    /// shortest leg, so it cannot swallow a neighbouring node.
    pub fn at(g: &RoadGraph, inc: &[usize], n: usize, class: impl Fn(usize) -> LaneClass) -> Option<Ring> {
        let cls: Vec<_> = inc.iter().map(|&e| class(e)).collect();
        let Junction::Roundabout(r) = junction(&cls) else { return None };
        let c = g.nodes[n];
        let other = |e: usize| g.nodes[if g.edges[e].a as usize == n { g.edges[e].b } else { g.edges[e].a } as usize];
        let shortest = inc.iter().map(|&e| other(e).distance(c)).fold(f32::INFINITY, f32::min);
        let mut legs: Vec<f32> = inc.iter().map(|&e| (other(e) - c).to_angle()).collect();
        legs.sort_by(f32::total_cmp);
        Some(Ring { c, r: r.min(shortest * 0.4).max(0.8), legs })
    }

    /// Polygon vertices around the ring: every leg plus the midpoint between neighbouring legs (ascending angle).
    fn verts(&self) -> Vec<f32> {
        let n = self.legs.len();
        let mut v = vec![];
        for i in 0..n {
            let (a, mut b) = (self.legs[i], self.legs[(i + 1) % n]);
            if b <= a {
                b += std::f32::consts::TAU;
            }
            v.extend([a, (a + b) / 2.0]);
        }
        v
    }

    /// Ring points a car drives from the leg toward `from` to the leg toward `to`, circulating (on screen) anticlockwise for
    /// right-hand traffic and clockwise for left-hand; see `ring_dir`. Shared vertices let cars on the ring and entering cars meet.
    pub fn path(&self, from: Vec2, to: Vec2, left_hand: bool) -> Vec<Vec2> {
        let v = self.verts();
        let near = |p: Vec2| {
            let a = (p - self.c).to_angle();
            (0..v.len()).step_by(2).min_by(|&i, &j| ang(v[i] - a).total_cmp(&ang(v[j] - a))).unwrap()
        };
        let (i0, i1) = (near(from), near(to));
        let step = if ring_dir(left_hand) > 0.0 { 1 } else { v.len() - 1 };
        let mut out = vec![];
        let mut i = i0;
        loop {
            out.push(self.c + Vec2::from_angle(v[i]) * self.r);
            i = (i + step) % v.len();
            if i == i1 {
                out.push(self.c + Vec2::from_angle(v[i]) * self.r);
                return out;
            }
        }
    }
}

fn ang(d: f32) -> f32 {
    let t = d.rem_euclid(std::f32::consts::TAU);
    t.min(std::f32::consts::TAU - t)
}

/// Ring circulation sign in world (x, y) maths terms: +1 anticlockwise, -1 clockwise. The camera looks down with
/// world +y toward the bottom of the screen, so the view is mirrored: maths-anticlockwise reads clockwise on screen.
/// Left-hand traffic circulates clockwise ON SCREEN (+1 here); right-hand anticlockwise on screen (-1).
pub fn ring_dir(left_hand: bool) -> f32 {
    if left_hand { 1.0 } else { -1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roadgen::{REdge, RoadClass};

    fn path(pts: &[Vec2]) -> RoadGraph {
        let edges = (0..pts.len() - 1)
            .map(|i| REdge { a: i as u32, b: i as u32 + 1, class: RoadClass::Arterial, bridge: false, len: 0.0, town: 0, order: 0 })
            .collect();
        RoadGraph { nodes: pts.to_vec(), edges }
    }

    /// R31: a fast road's sharp corner is re-aligned to at least its class radius; a street keeps it; a nearly
    /// straight through node is merged onto the line.
    #[test]
    fn fast_roads_meet_their_minimum_radius_and_collinear_nodes_merge() {
        let corner = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        for c in [LaneClass::FourLane, LaneClass::TenLane] {
            let mut g = path(&corner);
            assert!(!smooth(&mut g, |_| c, |_| true).is_empty());
            let r = kink_radius(g.nodes[0], g.nodes[1], g.nodes[2]);
            assert!(r >= min_radius(c), "{c:?}: radius {r} < {}", min_radius(c));
        }
        let mut g = path(&corner);
        assert!(smooth(&mut g, |_| LaneClass::Street, |_| true).is_empty(), "streets may turn sharply");
        let mut g = path(&[Vec2::ZERO, Vec2::new(10.0, 0.6), Vec2::new(20.0, 0.0)]);
        smooth(&mut g, |_| LaneClass::Street, |_| true);
        assert!(g.nodes[1].y.abs() < 1e-4 && (g.edges[0].len - 10.0).abs() < 1e-3, "kinked by ~7 degrees: merged onto the line");
    }

    /// R31: freeways cross other roads grade-separated; every other junction is a roundabout growing with its roads.
    #[test]
    fn freeways_grade_separate_and_junctions_are_roundabouts() {
        use LaneClass::*;
        assert_eq!(junction(&[SixLane, SixLane, TwoLaneLow]), Junction::Overpass);
        assert_eq!(junction(&[TenLane, Street, Street, TwoLaneHigh]), Junction::Overpass);
        let (Junction::Roundabout(small), Junction::Roundabout(big)) = (junction(&[Street; 3]), junction(&[FourLane, TwoLaneLow, TwoLaneLow])) else { panic!() };
        assert!(big > small);
        assert_eq!(junction(&[FourLane, FourLane]), Junction::None);
        assert_eq!(ring_dir(false), -ring_dir(true));
    }

    /// R31: a car crossing a four-leg roundabout goes round it anticlockwise (right-hand), clockwise (left-hand), as seen on
    /// screen (world maths sense is mirrored, see `ring_dir`).
    #[test]
    fn ring_path_circulates_with_the_driving_side() {
        let ring = Ring { c: Vec2::ZERO, r: 3.0, legs: vec![-1.5707964, 0.0, 1.5707964, 3.1415927] };
        for left in [false, true] {
            let p = ring.path(Vec2::new(12.0, 0.0), Vec2::new(-12.0, 0.0), left);
            assert_eq!(p.len(), 5, "half way round via the midpoints and the side leg");
            let turn: f32 = p.windows(2).map(|w| w[0].perp_dot(w[1])).sum();
            assert!(if left { turn > 0.0 } else { turn < 0.0 }, "left={left}: wrong way round");
            assert!(p.iter().all(|q| (q.length() - 3.0).abs() < 1e-4));
        }
    }
}
