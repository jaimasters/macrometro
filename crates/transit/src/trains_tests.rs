use super::*;
use sim_core::AgencyId;

const AG: AgencyId = AgencyId(1);

fn cmd(a: TransitAction) -> TransitCommand {
    TransitCommand { agency: AG, action: a }
}

/// `n` stops in a row, `len` apart, rail edges with `tracks`, one line over all.
fn line_net(n: u32, len: f64, tracks: u8, class: RailClass) -> Network {
    let mut net = Network::new(NetParams::default());
    for _ in 0..n {
        net.apply(&cmd(TransitAction::AddNode)).unwrap();
    }
    for i in 0..n - 1 {
        net.apply(&cmd(TransitAction::AddRail { a: NodeId(i), b: NodeId(i + 1), len, tracks, vcap: f64::INFINITY, crossings: vec![] })).unwrap();
    }
    let stops = (0..n).map(NodeId).collect();
    net.apply(&cmd(TransitAction::AddLine { class, stops, alignment_mismatch: 0.0 })).unwrap();
    net
}

fn loop_net(n: u32, len: f64) -> Network {
    let mut net = Network::new(NetParams::default());
    for _ in 0..n {
        net.apply(&cmd(TransitAction::AddNode)).unwrap();
    }
    for i in 0..n {
        net.apply(&cmd(TransitAction::AddRail { a: NodeId(i), b: NodeId((i + 1) % n), len, tracks: 1, vcap: f64::INFINITY, crossings: vec![] })).unwrap();
    }
    let mut stops: Vec<NodeId> = (0..n).map(NodeId).collect();
    stops.push(NodeId(0));
    net.apply(&cmd(TransitAction::AddLine { class: RailClass::LightRail, stops, alignment_mismatch: 0.0 })).unwrap();
    net
}

fn run(net: &Network, secs: f64) -> TrainSim {
    let mut s = TrainSim::default();
    for _ in 0..(secs / DT) as u64 {
        s.step(net);
    }
    s
}

#[test]
fn one_track_rejects_the_opposite_direction() {
    let mut net = Network::new(NetParams::default());
    for _ in 0..3 {
        net.apply(&cmd(TransitAction::AddNode)).unwrap();
    }
    for (a, b) in [(0, 1), (1, 2), (2, 0)] {
        net.apply(&cmd(TransitAction::AddRail { a: NodeId(a), b: NodeId(b), len: 30.0, tracks: 1, vcap: f64::INFINITY, crossings: vec![] })).unwrap();
    }
    let line = |stops: Vec<u32>| cmd(TransitAction::AddLine { class: RailClass::LightRail, stops: stops.into_iter().map(NodeId).collect(), alignment_mismatch: 0.0 });
    // reverse loop 0->2->1->0 runs against every one-way edge
    assert!(matches!(net.apply(&line(vec![0, 2, 1, 0])), Err(TransitError::WrongWay(..))));
    // a non-loop line cannot use one-way track at all (it has to come back)
    assert!(matches!(net.apply(&line(vec![0, 1, 2])), Err(TransitError::OneWayNeedsLoop(_))));
    assert!(net.apply(&line(vec![0, 1, 2, 0])).is_ok());
    // upgrading to 2 tracks makes the reverse legal
    for i in 0..3 {
        let (a, b) = [(0, 1), (1, 2), (2, 0)][i];
        net.apply(&cmd(TransitAction::UpgradeTracks { a: NodeId(a), b: NodeId(b) })).unwrap();
    }
    assert!(net.apply(&line(vec![0, 2, 1, 0])).is_ok());
    let up = cmd(TransitAction::UpgradeTracks { a: NodeId(0), b: NodeId(1) });
    assert!(net.apply(&up).is_ok()); // 2 -> 4
    assert!(matches!(net.apply(&up), Err(TransitError::MaxTracks(0))));
    assert_eq!(net.edges[0].upgrade_cost(), None);
    assert_eq!(net.edges[1].upgrade_cost(), Some(30.0 * TRACK_COST_PER_LEN * 2.0));
}

#[test]
fn one_way_loop_runs_around() {
    let net = loop_net(4, 40.0);
    let s = run(&net, 600.0);
    assert!(!s.trains.is_empty());
    // trains keep circulating forward (never reverse) and visit every stop: count dwell arrivals per node
    assert!(s.trains.iter().all(|t| t.dir == 1));
    let laps: f64 = s.trains.iter().map(|t| t.k as f64).sum();
    assert!(laps > 0.0);
    let mut s2 = TrainSim::default();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..(600.0 / DT) as u64 {
        s2.step(&net);
        for t in &s2.trains {
            if let Some((n, _)) = s2.on_platform(t) {
                seen.insert(n.0);
            }
        }
    }
    assert_eq!(seen.len(), 4, "every loop stop was served: {seen:?}");
}

/// Time for a lone train to leave stop 0 and be dwelling at stop 1.
fn first_leg(net: &Network) -> (f64, f64) {
    let mut s = TrainSim::default();
    let mut depart = None;
    for tick in 0..(400.0 / DT) as u64 {
        s.step(net);
        let t = &s.trains[0];
        if depart.is_none() && t.phase == Phase::Run && t.v > 0.0 {
            depart = Some(tick);
        }
        if depart.is_some() && t.k == 1 && matches!(t.phase, Phase::Dwell(_)) {
            return ((tick - depart.unwrap()) as f64 * DT, t.p);
        }
    }
    panic!("never arrived");
}

#[test]
fn stop_is_on_the_platform_mark_and_travel_time_matches_the_profile() {
    for (len, class) in [(120.0, RailClass::LightRail), (30.0, RailClass::LightRail), (200.0, RailClass::HeavyRail)] {
        let net = line_net(2, len, 2, class);
        let (t, p) = first_leg(&net);
        assert!((p - len).abs() <= STOP_EPS + 1e-9, "stopped at {p}, mark {len}");
        let tt = train_type(class);
        // analytic: accel at a to vmax, brake at the planned rate (PLAN * brake), cruise between; triangular if too short
        let b = tt.brake * PLAN;
        let (da, db) = (tt.vmax * tt.vmax / (2.0 * tt.accel), tt.vmax * tt.vmax / (2.0 * b));
        let want = if da + db <= len {
            tt.vmax / tt.accel + tt.vmax / b + (len - da - db) / tt.vmax
        } else {
            let vp = (2.0 * len * tt.accel * b / (tt.accel + b)).sqrt();
            vp / tt.accel + vp / b
        };
        assert!((t - want).abs() < want * 0.04 + 0.2, "len {len}: sim {t:.2}s vs analytic {want:.2}s");
    }
}

#[test]
fn a_train_never_overlaps_the_one_ahead_and_runs_are_identical() {
    let net = line_net(5, 60.0, 2, RailClass::LightRail);
    let go = || {
        let mut s = TrainSim::default();
        let mut min_gap = f64::INFINITY;
        for _ in 0..(900.0 / DT) as u64 {
            s.step(&net);
            for a in &s.trains {
                for b in s.trains.iter().filter(|b| b.id != a.id && b.dir == a.dir && b.p > a.p) {
                    min_gap = min_gap.min(b.p - b.len - a.p);
                }
            }
        }
        (min_gap, s.trains.iter().map(|t| (t.p.to_bits(), t.v.to_bits(), t.k)).collect::<Vec<_>>())
    };
    let (g1, r1) = go();
    let (g2, r2) = go();
    assert!(g1 >= 0.0, "trains overlapped: gap {g1}");
    assert_eq!(r1, r2, "runs differ");
    assert_eq!(g1, g2);
}

/// Three lines meet at hub node 3 and their trains arrive together: a single platform makes arrivals hold.
fn hub_run(platforms: u8) -> (u32, bool, f64) {
    let mut net = Network::new(NetParams::default());
    for _ in 0..4 {
        net.apply(&cmd(TransitAction::AddNode)).unwrap();
    }
    for i in 0..3 {
        net.apply(&cmd(TransitAction::AddRail { a: NodeId(i), b: NodeId(3), len: 60.0, tracks: 2, vcap: f64::INFINITY, crossings: vec![] })).unwrap();
        net.apply(&cmd(TransitAction::AddLine { class: RailClass::LightRail, stops: vec![NodeId(i), NodeId(3)], alignment_mismatch: 0.0 })).unwrap();
    }
    for _ in 1..platforms {
        net.apply(&cmd(TransitAction::AddPlatform(NodeId(3)))).unwrap();
    }
    let mut s = TrainSim::default();
    let (mut served, mut held, mut was) = (0, false, vec![]);
    let mut first_all = None;
    for tick in 0..(300.0 / DT) as u64 {
        s.step(&net);
        let at_hub = |t: &Train| matches!(t.phase, Phase::Dwell(_)) && t.slot.is_some_and(|(n, _)| n == NodeId(3));
        for t in &s.trains {
            if at_hub(t) && !was.contains(&t.id) {
                served += 1;
                was.push(t.id);
            }
            if !at_hub(t) {
                was.retain(|x| *x != t.id);
            }
            held |= t.phase == Phase::Run && t.v == 0.0 && t.slot.is_none() && t.k == 1 && t.p > 1.0;
        }
        if first_all.is_none() && served >= 3 {
            first_all = Some(tick as f64 * DT);
        }
    }
    (served, held, first_all.unwrap_or(f64::INFINITY))
}

#[test]
fn full_station_holds_arrivals_and_a_platform_raises_throughput() {
    let (_, held1, t1) = hub_run(1);
    let (_, held3, t3) = hub_run(3);
    assert!(held1, "no train held for the single platform");
    assert!(!held3, "three platforms should not hold three arrivals");
    assert!(t3 < t1, "all three served by {t3}s with 3 platforms vs {t1}s with 1");
}

#[test]
fn level_crossing_closes_the_road_while_a_train_passes_a_bridge_does_not() {
    let mk = |kind| {
        let mut net = Network::new(NetParams::default());
        for _ in 0..2 {
            net.apply(&cmd(TransitAction::AddNode)).unwrap();
        }
        net.apply(&cmd(TransitAction::AddRail { a: NodeId(0), b: NodeId(1), len: 100.0, tracks: 2, vcap: f64::INFINITY, crossings: vec![Crossing { kind, at: 50.0, road: 7 }] })).unwrap();
        net.apply(&cmd(TransitAction::AddLine { class: RailClass::LightRail, stops: vec![NodeId(0), NodeId(1)], alignment_mismatch: 0.0 })).unwrap();
        net
    };
    let closed_at_some_point = |net: &Network| {
        let mut s = TrainSim::default();
        let mut hit = false;
        let mut frac = 0.0;
        for _ in 0..(120.0 / DT) as u64 {
            s.step(net);
            hit |= s.level_crossings_occupied(net).iter().any(|c| c.0 == 7);
        }
        if let Some(f) = s.take_closure().get(&7) {
            frac = *f;
        }
        (hit, frac)
    };
    let (hit, frac) = closed_at_some_point(&mk(CrossingKind::Level));
    assert!(hit && frac > 0.0 && frac < 1.0, "level: hit {hit} frac {frac}");
    assert_eq!(closed_at_some_point(&mk(CrossingKind::Bridge)), (false, 0.0));
    assert_eq!(closed_at_some_point(&mk(CrossingKind::Underpass)), (false, 0.0));
}

#[test]
fn a_fast_train_overtakes_a_slow_one_only_on_four_tracks() {
    // returns the line that first reaches stop 1, and the smallest bumper gap ever seen on one lane
    let go = |tracks| {
        let mut net = Network::new(NetParams::default());
        for _ in 0..3 {
            net.apply(&cmd(TransitAction::AddNode)).unwrap();
        }
        for i in 0..2 {
            net.apply(&cmd(TransitAction::AddRail { a: NodeId(i), b: NodeId(i + 1), len: 1200.0, tracks, vcap: f64::INFINITY, crossings: vec![] })).unwrap();
        }
        let l = |class| cmd(TransitAction::AddLine { class, stops: vec![NodeId(0), NodeId(1), NodeId(2)], alignment_mismatch: 0.0 });
        net.apply(&l(RailClass::Tram)).unwrap();
        net.apply(&l(RailClass::HighSpeed)).unwrap();
        let mut s = TrainSim::default();
        let (mut first, mut min_gap) = (None, f64::INFINITY);
        for _ in 0..(400.0 / DT) as u64 {
            s.step(&net);
            for t in s.trains.iter().filter(|t| t.k == 1 && matches!(t.phase, Phase::Dwell(_))) {
                first.get_or_insert(t.line);
            }
            for a in &s.trains {
                for b in s.trains.iter().filter(|b| b.id != a.id && b.dir == a.dir && b.p > a.p && b.lane == a.lane) {
                    let g = b.p - b.len - a.p;
                    if g < min_gap && g < 0.0 && std::env::var("DBG").is_ok() {
                        eprintln!("tracks {tracks} tick {} a={:?} b={:?}", s.tick, a, b);
                    }
                    min_gap = min_gap.min(g);
                }
            }
        }
        (first, min_gap)
    };
    let (f2, g2) = go(2);
    let (f4, g4) = go(4);
    assert_eq!(f2, Some(LineId(0)), "2 tracks: the slow tram stays in front");
    assert_eq!(f4, Some(LineId(1)), "4 tracks: the express overtakes");
    assert!(g2 >= 0.0 && g4 >= 0.0, "overlap: {g2} {g4}");
}

#[test]
fn opposing_trains_share_a_single_track_and_meet_at_station_loops_without_deadlock() {
    let mut net = Network::new(NetParams::default());
    for _ in 0..3 {
        net.apply(&cmd(TransitAction::AddNode)).unwrap();
    }
    for i in 0..2 {
        net.apply(&cmd(TransitAction::AddSingleRail { a: NodeId(i), b: NodeId(i + 1), len: 150.0, vcap: f64::INFINITY, crossings: vec![] })).unwrap();
    }
    for n in 0..3 {
        net.apply(&cmd(TransitAction::AddPlatform(NodeId(n)))).unwrap(); // island: two platform tracks
    }
    net.apply(&cmd(TransitAction::AddLine { class: RailClass::LightRail, stops: (0..3).map(NodeId).collect(), alignment_mismatch: 0.0 })).unwrap();
    let mut s = TrainSim::default();
    let (mut went_back, mut reached_end) = (false, false);
    for _ in 0..(900.0 / DT) as u64 {
        s.step(&net);
        for t in &s.trains {
            went_back |= t.dir < 0;
            reached_end |= t.dir < 0 && t.k >= 2;
        }
        // never two opposing trains moving on the same single-track edge
        for a in &s.trains {
            for b in &s.trains {
                if a.id < b.id && a.dir != b.dir && a.phase == Phase::Run && b.phase == Phase::Run && a.v > 0.0 && b.v > 0.0 {
                    assert_ne!(s.edge_under(a), s.edge_under(b), "head-on at tick {}", s.tick);
                }
            }
        }
    }
    assert!(s.trains.len() >= 2, "several trains share the line");
    assert!(went_back && reached_end, "trains complete the out-and-back run: no deadlock");
}

#[test]
fn passengers_wait_board_up_to_capacity_and_get_off() {
    let net = line_net(3, 100.0, 2, RailClass::LightRail);
    let mut s = TrainSim::default();
    s.arrivals.insert(0, 1.0); // a person a second at the first station
    let mut peak: f64 = 0.0;
    let mut mid_gain = false;
    for _ in 0..(400.0 / DT) as u64 {
        s.step(&net);
        for t in &s.trains {
            peak = peak.max(t.load);
            assert!(t.load <= TRAIN_CAPACITY + 1e-9 && t.load >= 0.0);
            mid_gain |= t.k == 1 && t.load > 0.0 && matches!(t.phase, Phase::Dwell(_));
        }
    }
    assert!(peak > 5.0, "riders board at the origin: {peak}");
    assert!(mid_gain, "load survives into the next stop");
    assert!(s.waiting_at(NodeId(0)) < 400.0, "boarding drains the queue");
}
