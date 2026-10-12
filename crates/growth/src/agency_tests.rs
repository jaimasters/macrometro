use super::*;
use crate::core::Zone;

fn zones() -> Zones {
    let z = |id, pop| Zone { id: ZoneId(id), pop, jobs: 1.0, capacity: 1000.0, crowding: 0.0 };
    Zones::new(vec![z(0, 100.0), z(1, 200.0)])
}

fn p() -> AgencyParams {
    AgencyParams { s_min: 0.2, t_ticks: 5, lead_ticks: 10, cost_per_pop: 1.0 }
}

#[test]
fn underserved_for_t_ticks_emits_plan_ahead() {
    let (mut a, zs) = (RoadAgency::new(p()), zones());
    let mut plans = vec![];
    for t in 0..5 {
        plans.extend(a.observe(t, &zs, &[0.05, 0.9]).planned);
    }
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].zone, ZoneId(0));
    assert_eq!(plans[0].build_tick, plans[0].emitted_tick + 10);
    assert_eq!(plans[0].cost, 100.0);
}

#[test]
fn road_reserve_tracks_pending_then_clears_when_due() {
    let (mut a, zs) = (RoadAgency::new(p()), zones());
    let mut last = AgencyOutput::default();
    for t in 0..5 {
        last = a.observe(t, &zs, &[0.0, 0.9]);
    }
    assert_eq!(last.road_reserve, 100.0);
    let o = a.observe(13, &zs, &[0.9, 0.9]);
    assert_eq!(o.road_reserve, 100.0, "not yet due");
    let o = a.observe(14, &zs, &[0.9, 0.9]);
    assert_eq!(o.due.len(), 1);
    assert_eq!(o.road_reserve, 0.0);
}

#[test]
fn recovery_resets_streak_and_no_duplicate_while_pending() {
    let (mut a, zs) = (RoadAgency::new(p()), zones());
    for t in 0..4 {
        a.observe(t, &zs, &[0.0, 0.9]);
    }
    a.observe(4, &zs, &[0.5, 0.9]); // recovered: streak reset
    for t in 5..9 {
        assert!(a.observe(t, &zs, &[0.0, 0.9]).planned.is_empty());
    }
    assert_eq!(a.observe(9, &zs, &[0.0, 0.9]).planned.len(), 1);
    for t in 10..20 {
        assert!(a.observe(t, &zs, &[0.0, 0.9]).planned.is_empty(), "pending plan blocks duplicates");
    }
}

#[test]
fn agency_deterministic() {
    let run = || {
        let (mut a, zs) = (RoadAgency::new(p()), zones());
        (0..60u64)
            .flat_map(|t| {
                let s = if t % 7 == 0 { 0.5 } else { 0.0 };
                a.observe(t, &zs, &[s, 0.0]).planned
            })
            .map(|p| (p.zone, p.emitted_tick, p.build_tick, p.cost.to_bits()))
            .collect::<Vec<_>>()
    };
    let r = run();
    assert!(!r.is_empty());
    assert_eq!(r, run());
}
