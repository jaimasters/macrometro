use super::*;

fn z(id: u32, pop: f64, jobs: f64, cap: f64, crowding: f64) -> Zone {
    Zone { id: ZoneId(id), pop, jobs, capacity: cap, crowding }
}

#[test]
fn cost_matrix_get_set_infinity() {
    let mut c = CostMatrix::new(3);
    assert_eq!(c.get(1, 1), 0.0);
    assert!(c.get(0, 2).is_infinite());
    c.set(0, 2, 4.5);
    assert_eq!(c.get(0, 2), 4.5);
    assert!(c.get(2, 0).is_infinite());
}

#[test]
fn accessibility_decreases_with_cost() {
    let zones = Zones::new(vec![z(0, 1.0, 100.0, 1.0, 0.0), z(1, 1.0, 0.0, 1.0, 0.0)]);
    let near = CostMatrix::from_fn(2, |i, j| if i == j { 0.0 } else { 1.0 });
    let far = CostMatrix::from_fn(2, |i, j| if i == j { 0.0 } else { 10.0 });
    // zone 1 has no jobs of its own: A_1 is purely from zone 0
    assert!(accessibility(&zones, &near, 0.1)[1] > accessibility(&zones, &far, 0.1)[1]);
}

#[test]
fn accessibility_ignores_unreachable() {
    let zones = Zones::new(vec![z(0, 1.0, 10.0, 1.0, 0.0), z(1, 1.0, 1000.0, 1.0, 0.0)]);
    let c = CostMatrix::new(2);
    let a = accessibility(&zones, &c, 0.1);
    assert_eq!(a[0], 10.0);
    assert_eq!(a[1], 1000.0);
}

#[test]
fn accessibility_scales_with_jobs() {
    let c = CostMatrix::from_fn(2, |i, j| if i == j { 0.0 } else { 3.0 });
    let a1 = accessibility(&Zones::new(vec![z(0, 1.0, 10.0, 1.0, 0.0), z(1, 1.0, 10.0, 1.0, 0.0)]), &c, 0.2);
    let a2 = accessibility(&Zones::new(vec![z(0, 1.0, 20.0, 1.0, 0.0), z(1, 1.0, 20.0, 1.0, 0.0)]), &c, 0.2);
    assert!((a2[0] - 2.0 * a1[0]).abs() < 1e-9);
}

fn one_zone(pop: f64, cap: f64, crowding: f64) -> Zones {
    Zones::new(vec![z(0, pop, 100.0, cap, crowding)])
}

#[test]
fn growth_zero_when_full_capacity() {
    let zs = one_zone(1000.0, 1000.0, 0.0);
    let d = growth_deltas(&zs, &[500.0], &GrowthParams::default());
    assert_eq!(d[0].dpop, 0.0);
}

#[test]
fn growth_reduced_by_crowding() {
    let p = GrowthParams::default();
    let calm = growth_deltas(&one_zone(100.0, 1000.0, 0.5), &[100.0], &p)[0].dpop;
    let jam = growth_deltas(&one_zone(100.0, 1000.0, 2.0), &[100.0], &p)[0].dpop;
    assert!(jam < calm && jam > 0.0);
    assert!((jam - calm / 2.0).abs() < 1e-9, "load factor 2 halves growth");
}

#[test]
fn growth_increases_with_accessibility() {
    let p = GrowthParams::default();
    let lo = growth_deltas(&one_zone(100.0, 1000.0, 0.0), &[10.0], &p)[0].dpop;
    let hi = growth_deltas(&one_zone(100.0, 1000.0, 0.0), &[500.0], &p)[0].dpop;
    assert!(hi > lo);
}

#[test]
fn apply_deltas_sorted_order_deterministic() {
    let base = Zones::new(vec![z(0, 10.0, 5.0, 100.0, 0.0), z(1, 20.0, 5.0, 100.0, 0.0), z(2, 30.0, 5.0, 100.0, 0.0)]);
    let ds: Vec<ZoneDelta> = (0..3).map(|i| ZoneDelta { id: ZoneId(i), dpop: 0.1 * (i + 1) as f64, djobs: 0.01 }).collect();
    let mut rev = ds.clone();
    rev.reverse();
    let (mut a, mut b) = (base.clone(), base);
    apply_deltas(&mut a, &ds);
    apply_deltas(&mut b, &rev);
    assert_eq!(a, b);
}

#[test]
fn apply_deltas_clamps_capacity() {
    let mut zs = one_zone(99.0, 100.0, 0.0);
    apply_deltas(&mut zs, &[ZoneDelta { id: ZoneId(0), dpop: 50.0, djobs: 0.0 }]);
    assert_eq!(zs.0[0].pop, 100.0);
}

fn world() -> (Zones, CostMatrix) {
    let zs = Zones::new(vec![
        z(0, 100.0, 50.0, 1000.0, 0.0),
        z(1, 10.0, 200.0, 800.0, 0.3),
        z(2, 40.0, 20.0, 500.0, 1.5),
    ]);
    (zs, CostMatrix::from_fn(3, |i, j| if i == j { 0.0 } else { (i + j) as f64 * 3.0 }))
}

#[test]
fn same_inputs_same_hash() {
    let run = || {
        let (zs, c) = world();
        let mut s = GrowthSim::new(zs, GrowthParams::default());
        for _ in 0..50 {
            s.step(&c);
        }
        s.state_hash()
    };
    assert_eq!(run(), run());
    let (zs, c) = world();
    let mut s = GrowthSim::new(zs, GrowthParams::default());
    let h0 = s.state_hash();
    s.step(&c);
    assert_ne!(h0, s.state_hash());
}

#[test]
fn growth_converges_toward_capacity() {
    let (zs, c) = world();
    let mut s = GrowthSim::new(zs, GrowthParams { rate: 0.2, ..Default::default() });
    for _ in 0..2000 {
        s.step(&c);
    }
    for zn in &s.zones.0 {
        assert!(zn.pop <= zn.capacity);
    }
    // uncrowded zone 0 ends within 10% of capacity
    assert!(s.zones.0[0].pop > 900.0, "{}", s.zones.0[0].pop);
}
