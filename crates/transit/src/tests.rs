use super::*;
use sim_core::AgencyId;

const AG: AgencyId = AgencyId(1);
const OTHER: AgencyId = AgencyId(2);

fn cmd(agency: AgencyId, action: TransitAction) -> TransitCommand {
    TransitCommand { agency, action }
}

/// Nodes 0..n, road+rail edges between neighbours of length `len`; one `class` line over all.
fn corridor(n: u32, len: f64, class: RailClass, align: f64, with_line: bool) -> Network {
    let mut net = Network::new(NetParams::default());
    for _ in 0..n {
        net.apply(&cmd(AG, TransitAction::AddNode)).unwrap();
    }
    for i in 0..n - 1 {
        for kind in [EdgeKind::Road, EdgeKind::Rail] {
            net.apply(&cmd(AG, TransitAction::AddEdge { a: NodeId(i), b: NodeId(i + 1), len, kind })).unwrap();
        }
    }
    if with_line {
        let stops = (0..n).map(NodeId).collect();
        net.apply(&cmd(AG, TransitAction::AddLine { class, stops, alignment_mismatch: align })).unwrap();
    }
    net
}

fn nodes(n: u32) -> Vec<NodeId> {
    (0..n).map(NodeId).collect()
}

#[test]
fn penalty_zero_when_matched() {
    for c in [RailClass::Tram, RailClass::LightRail, RailClass::HeavyRail, RailClass::HighSpeed, RailClass::DoubleDecker] {
        assert!(mismatch_penalty(c, class_profile(c).1, 0.0).abs() < 1e-12);
    }
}

#[test]
fn penalty_capped_at_30_percent() {
    assert!((mismatch_penalty(RailClass::Tram, 1e6, 1.0) - 0.30).abs() < 1e-12);
    assert!((mismatch_penalty(RailClass::Tram, 1e-6, 5.0) - 0.30).abs() < 1e-12);
    for s in [0.01, 0.4, 3.0, 100.0] {
        for a in [0.0, 0.3, 1.0] {
            let p = mismatch_penalty(RailClass::HeavyRail, s, a);
            assert!((0.0..=0.30 + 1e-12).contains(&p));
        }
    }
}

#[test]
fn penalty_monotone_in_spacing_mismatch() {
    let ideal = class_profile(RailClass::LightRail).1;
    let p = |s| mismatch_penalty(RailClass::LightRail, s, 0.0);
    assert!(p(ideal * 1.5) > p(ideal * 1.1));
    assert!(p(ideal / 2.0) > p(ideal / 1.2));
}

#[test]
fn penalty_monotone_in_alignment_mismatch() {
    let ideal = class_profile(RailClass::LightRail).1;
    assert!(mismatch_penalty(RailClass::LightRail, ideal, 0.8) > mismatch_penalty(RailClass::LightRail, ideal, 0.2));
}

#[test]
fn penalty_scales_speed_and_capacity() {
    let (v, _, cap) = class_profile(RailClass::HeavyRail);
    assert!((effective_speed(RailClass::HeavyRail, 0.3) - 0.7 * v).abs() < 1e-9);
    assert!((effective_capacity(RailClass::HeavyRail, 0.3) - 0.7 * cap).abs() < 1e-9);
}

#[test]
fn matched_spacing_gives_cheaper_trip_than_mismatched() {
    let good = corridor(3, 2.0, RailClass::HeavyRail, 0.0, true);
    let bad = corridor(3, 2.0, RailClass::HeavyRail, 1.0, true);
    assert!(cost_matrix(&good, &nodes(3)).get(0, 2) < cost_matrix(&bad, &nodes(3)).get(0, 2));
}

#[test]
fn cost_matrix_zero_diag_and_symmetric() {
    let c = cost_matrix(&corridor(4, 2.0, RailClass::HeavyRail, 0.0, true), &nodes(4));
    assert_eq!(c.n, 4);
    for i in 0..4 {
        assert_eq!(c.get(i, i), 0.0);
        for j in 0..4 {
            assert_eq!(c.get(i, j), c.get(j, i));
        }
    }
}

#[test]
fn cost_matrix_unreachable_is_infinite() {
    let mut net = corridor(2, 1.0, RailClass::Tram, 0.0, false);
    net.apply(&cmd(AG, TransitAction::AddNode)).unwrap();
    assert_eq!(cost_matrix(&net, &nodes(3)).get(0, 2), f64::INFINITY);
}

#[test]
fn cost_matrix_prefers_faster_rail_over_road() {
    let road_only = corridor(2, 20.0, RailClass::HeavyRail, 0.0, false);
    let railed = corridor(2, 20.0, RailClass::HeavyRail, 0.0, true);
    let (r, t) = (cost_matrix(&road_only, &nodes(2)).get(0, 1), cost_matrix(&railed, &nodes(2)).get(0, 1));
    assert!((r - 20.0 / 30.0).abs() < 1e-12);
    assert!(t < r);
}

#[test]
fn unserved_rail_edge_is_unusable() {
    let mut net = Network::new(NetParams::default());
    for _ in 0..2 {
        net.apply(&cmd(AG, TransitAction::AddNode)).unwrap();
    }
    net.apply(&cmd(AG, TransitAction::AddEdge { a: NodeId(0), b: NodeId(1), len: 1.0, kind: EdgeKind::Rail })).unwrap();
    assert_eq!(cost_matrix(&net, &nodes(2)).get(0, 1), f64::INFINITY);
}

#[test]
fn cost_matrix_deterministic() {
    let net = corridor(6, 1.5, RailClass::LightRail, 0.3, true);
    assert_eq!(cost_matrix(&net, &nodes(6)), cost_matrix(&net, &nodes(6)));
}

#[test]
fn cost_matrix_feeds_growth_accessibility() {
    let zones = growth::Zones::new(
        (0..3).map(|i| growth::Zone { id: growth::ZoneId(i), pop: 10.0, jobs: 100.0, capacity: 1e3, crowding: 0.0 }).collect(),
    );
    let slow = growth::accessibility(&zones, &cost_matrix(&corridor(3, 20.0, RailClass::HeavyRail, 0.0, false), &nodes(3)), 0.1);
    let fast = growth::accessibility(&zones, &cost_matrix(&corridor(3, 20.0, RailClass::HeavyRail, 0.0, true), &nodes(3)), 0.1);
    assert!(fast[0] > slow[0]);
}

#[test]
fn commands_reject_unknown_node_bad_len_and_short_line() {
    let mut net = Network::new(NetParams::default());
    net.apply(&cmd(AG, TransitAction::AddNode)).unwrap();
    let e = |len| cmd(AG, TransitAction::AddEdge { a: NodeId(0), b: NodeId(5), len, kind: EdgeKind::Road });
    assert_eq!(net.apply(&e(1.0)), Err(TransitError::UnknownNode(NodeId(5))));
    let e2 = cmd(AG, TransitAction::AddEdge { a: NodeId(0), b: NodeId(0), len: -1.0, kind: EdgeKind::Road });
    assert_eq!(net.apply(&e2), Err(TransitError::BadLength));
    let l = cmd(AG, TransitAction::AddLine { class: RailClass::Tram, stops: vec![NodeId(0)], alignment_mismatch: 0.0 });
    assert_eq!(net.apply(&l), Err(TransitError::TooFewStops));
}

#[test]
fn line_requires_rail_edges() {
    let mut net = corridor(2, 1.0, RailClass::Tram, 0.0, false);
    net.edges.retain(|e| e.kind == EdgeKind::Road);
    let l = cmd(AG, TransitAction::AddLine { class: RailClass::Tram, stops: nodes(2), alignment_mismatch: 0.0 });
    assert_eq!(net.apply(&l), Err(TransitError::NoRailEdge(NodeId(0), NodeId(1))));
}

#[test]
fn commands_carry_agency_and_only_owner_removes_line() {
    let mut net = corridor(3, 1.0, RailClass::Tram, 0.0, true);
    assert!(net.edges.iter().all(|e| e.owner == AG));
    let id = net.lines[0].id;
    assert_eq!(net.lines[0].owner, AG);
    assert_eq!(net.apply(&cmd(OTHER, TransitAction::RemoveLine(id))), Err(TransitError::NotOwner));
    assert_eq!(net.apply(&cmd(AG, TransitAction::RemoveLine(id))), Ok(Applied::Removed));
    assert_eq!(net.apply(&cmd(AG, TransitAction::RemoveLine(id))), Err(TransitError::UnknownLine(id)));
}

#[test]
fn transit_share_zero_without_rail_and_rises_with_it() {
    let w = [1.0; 3];
    let none = transit_shares(&corridor(3, 20.0, RailClass::HeavyRail, 0.0, false), &nodes(3), &w);
    assert!(none.iter().all(|&s| s == 0.0));
    let good = transit_shares(&corridor(3, 20.0, RailClass::HeavyRail, 0.0, true), &nodes(3), &w);
    let bad = transit_shares(&corridor(3, 20.0, RailClass::Tram, 1.0, true), &nodes(3), &w);
    assert!(good.iter().all(|s| (0.0..=1.0).contains(s)));
    assert!(good[0] > bad[0]);
}

#[test]
fn plugin_publishes_costs_and_shares_to_growth() {
    let mut app = App::new();
    app.add_plugins((growth::GrowthPlugin, TransitPlugin));
    let net = corridor(3, 20.0, RailClass::HeavyRail, 0.0, true);
    app.world_mut().resource_mut::<TransitNet>().0 = net;
    app.world_mut().resource_mut::<ZoneNodes>().0 = nodes(3);
    app.world_mut().resource_mut::<TransitCommands>().0 = vec![cmd(OTHER, TransitAction::RemoveLine(LineId(99)))];
    app.world_mut().run_schedule(FixedUpdate);
    let c = app.world().resource::<growth::Costs>().0.clone().expect("costs published");
    assert_eq!(c.n, 3);
    assert!(c.get(0, 2).is_finite());
    assert_eq!(app.world().resource::<TransitErrors>().0.len(), 1);
    assert_eq!(app.world().resource::<TransitShares>().0.len(), 3);
}

#[test]
fn remove_node_drops_segments_bridges_and_prunes_short_lines() {
    // 0-1-2-3 line; removing interior node 1 bridges 0-2; removing the end node 3 shortens it.
    let mut net = corridor(4, 10.0, RailClass::Tram, 0.0, true);
    assert_eq!(net.apply(&cmd(AG, TransitAction::RemoveNode(NodeId(1)))), Ok(Applied::Removed));
    assert!(net.edges.iter().all(|e| e.a != NodeId(1) && e.b != NodeId(1)));
    assert_eq!(net.lines[0].stops, vec![NodeId(0), NodeId(2), NodeId(3)]);
    let bridge = &net.edges[net.lines[0].segments[0]];
    assert_eq!((bridge.kind, bridge.len), (EdgeKind::Rail, 20.0));
    assert_eq!(net.lines[0].segments.len(), 2);
    assert_eq!(net.apply(&cmd(OTHER, TransitAction::RemoveNode(NodeId(3)))), Err(TransitError::NotOwner));
    net.apply(&cmd(AG, TransitAction::RemoveNode(NodeId(3)))).unwrap();
    assert_eq!(net.lines[0].stops.len(), 2);
    net.apply(&cmd(AG, TransitAction::RemoveNode(NodeId(0)))).unwrap();
    assert!(net.lines.is_empty(), "line below 2 stops is removed");
    assert_eq!(net.apply(&cmd(AG, TransitAction::RemoveNode(NodeId(99)))), Err(TransitError::UnknownNode(NodeId(99))));
}

#[test]
fn extend_line_at_either_terminus_only_over_a_rail_edge() {
    let mut net = corridor(4, 10.0, RailClass::LightRail, 0.0, false);
    net.apply(&cmd(AG, TransitAction::AddLine { class: RailClass::LightRail, stops: vec![NodeId(1), NodeId(2)], alignment_mismatch: 0.0 })).unwrap();
    let id = net.lines[0].id;
    let ext = |from: u32, to: u32| cmd(AG, TransitAction::ExtendLine { line: id, from: NodeId(from), to: NodeId(to) });
    net.apply(&ext(2, 3)).unwrap();
    net.apply(&ext(1, 0)).unwrap();
    assert_eq!(net.lines[0].stops, vec![NodeId(0), NodeId(1), NodeId(2), NodeId(3)]);
    assert_eq!(net.lines[0].segments.len(), 3);
    assert_eq!(net.apply(&ext(1, 2)), Err(TransitError::NotTerminus(NodeId(1))));
    assert_eq!(net.apply(&ext(3, 3)), Err(TransitError::NoRailEdge(NodeId(3), NodeId(3))));
    assert_eq!(net.apply(&cmd(OTHER, TransitAction::ExtendLine { line: id, from: NodeId(3), to: NodeId(2) })), Err(TransitError::NotOwner));
    assert_eq!(net.apply(&cmd(AG, TransitAction::ExtendLine { line: LineId(9), from: NodeId(0), to: NodeId(1) })), Err(TransitError::UnknownLine(LineId(9))));
}

#[test]
fn platforms_start_at_one_and_cap() {
    let mut net = corridor(2, 10.0, RailClass::LightRail, 0.0, false);
    assert_eq!(net.platform_count(NodeId(1)), 1);
    for want in 2..=MAX_PLATFORMS {
        net.apply(&cmd(AG, TransitAction::AddPlatform(NodeId(1)))).unwrap();
        assert_eq!(net.platform_count(NodeId(1)), want);
    }
    assert_eq!(net.apply(&cmd(AG, TransitAction::AddPlatform(NodeId(1)))), Err(TransitError::MaxPlatforms(NodeId(1))));
    assert_eq!(net.platform_count(NodeId(0)), 1);
    assert_eq!(net.apply(&cmd(AG, TransitAction::AddPlatform(NodeId(7)))), Err(TransitError::UnknownNode(NodeId(7))));
}
