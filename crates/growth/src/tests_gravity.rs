use super::*;

fn z(i: u32, pop: f64, jobs: f64) -> Zone {
    Zone { id: ZoneId(i), pop, jobs, capacity: 1000.0, crowding: 0.0 }
}

fn world() -> Zones {
    Zones::new(vec![z(0, 100.0, 50.0), z(1, 10.0, 200.0), z(2, 40.0, 20.0)])
}

#[test]
fn gravity_row_sums_equal_emitted() {
    let zones = world();
    let c = CostMatrix::from_fn(3, |i, j| if i == j { 0.0 } else { ((i + j) * 2) as f64 });
    let f = gravity_flows(&zones, &c, 0.1, 0.5);
    for i in 0..3 {
        assert!((f.row_sum(i) - zones.0[i].pop * 0.5).abs() < 1e-9, "row {i}");
    }
}

#[test]
fn gravity_zero_for_unreachable() {
    let zones = world();
    let mut c = CostMatrix::new(3);
    c.set(0, 1, 2.0);
    let f = gravity_flows(&zones, &c, 0.1, 0.5);
    assert_eq!(f.get(0, 2), 0.0);
    assert_eq!(f.get(1, 0), 0.0);
    assert!(f.get(0, 1) > 0.0);
    assert!(f.row_sum(2) > 0.0);
    assert_eq!(f.get(2, 2), f.row_sum(2));
}

#[test]
fn gravity_closer_gets_more() {
    let zones = Zones::new(vec![z(0, 100.0, 50.0), z(1, 100.0, 50.0), z(2, 100.0, 50.0)]);
    let mut c = CostMatrix::new(3);
    c.set(0, 1, 1.0);
    c.set(0, 2, 8.0);
    let f = gravity_flows(&zones, &c, 0.1, 0.5);
    assert!(f.get(0, 1) > f.get(0, 2));
}

#[test]
fn gravity_total_conserved() {
    let zones = world();
    let expected = zones.0.iter().map(|z| z.pop).sum::<f64>() * 0.5;
    for scale in [1.0, 3.0] {
        let c = CostMatrix::from_fn(3, move |i, j| {
            if i == j { 0.0 } else { scale * ((i + j) * 2) as f64 }
        });
        let f = gravity_flows(&zones, &c, 0.1, 0.5);
        assert!((f.total() - expected).abs() < 1e-9, "scale {scale}");
    }
}
