use super::*;

const A: AgencyId = AgencyId(1);
const B: AgencyId = AgencyId(2);

fn card(v: f64) -> Scorecard {
    Scorecard { accessibility: v, trip_time: v, transit_share: v, crowding: v, emissions: v, equity: v }
}

fn two() -> EconomyState {
    let mut s = EconomyState::new(EconomyParams::default());
    s.add_agency(A, 0.5);
    s.add_agency(B, 0.5);
    s
}

#[test]
fn next_budget_deterministic() {
    let s = two();
    let cards = [(A, card(0.7)), (B, card(0.4))];
    assert_eq!(next_budget(&s, &cards, 0.6, 0.0), next_budget(&s, &cards, 0.6, 0.0));
    // insertion order of agencies must not matter
    let mut s2 = EconomyState::new(EconomyParams::default());
    s2.add_agency(B, 0.5);
    s2.add_agency(A, 0.5);
    assert_eq!(next_budget(&s, &cards, 0.6, 0.0), next_budget(&s2, &cards, 0.6, 0.0));
}

#[test]
fn better_performer_larger_share() {
    let b = next_budget(&two(), &[(A, card(0.8)), (B, card(0.3))], 0.0, 0.0);
    assert!(b.per_agency[&A].amount > b.per_agency[&B].amount);
}

#[test]
fn favouritism_concentrates_with_car_share() {
    let cards = [(A, card(0.8)), (B, card(0.3))];
    let lo = next_budget(&two(), &cards, 0.0, 0.0);
    let hi = next_budget(&two(), &cards, 1.0, 0.0);
    let frac = |b: &Budget| b.per_agency[&A].amount / (b.per_agency[&A].amount + b.per_agency[&B].amount);
    assert!(frac(&hi) > frac(&lo));
}

#[test]
fn budget_conserves_total() {
    let b = next_budget(&two(), &[(A, card(0.8)), (B, card(0.3))], 0.4, 100.0);
    assert_eq!(b.road_reserve, 100.0);
    let sum: f64 = b.per_agency.values().map(|x| x.amount).sum::<f64>() + b.road_reserve;
    assert!((sum - b.total).abs() < 1e-9);
}

#[test]
fn road_reserve_competes_with_agency_pool() {
    let cards = [(A, card(0.5)), (B, card(0.5))];
    let calm = next_budget(&two(), &cards, 0.0, 0.0);
    let under = next_budget(&two(), &cards, 0.0, 200.0);
    assert_eq!(calm.road_reserve, 0.0);
    assert_eq!(under.road_reserve, 200.0);
    // demand above the cap is clamped: city never takes more than max_road_reserve of total
    let huge = next_budget(&two(), &cards, 0.0, 1e9);
    assert!((huge.road_reserve - huge.total * EconomyParams::default().max_road_reserve).abs() < 1e-9);
    assert!(under.per_agency[&A].amount < calm.per_agency[&A].amount);
}

#[test]
fn earmark_rises_with_car_share() {
    let cards = [(A, card(0.5)), (B, card(0.5))];
    let share = |car| {
        next_budget(&two(), &cards, car, 0.0).per_agency[&A]
            .conditions
            .iter()
            .find(|c| c.category == Category::RoadAdjacent)
            .map_or(0.0, |c| c.min_share)
    };
    assert_eq!(share(0.0), 0.0);
    assert!(share(0.9) > share(0.3));
}

fn with_condition() -> (EconomyState, Budget) {
    let mut s = two();
    s.agencies.get_mut(&A).unwrap().conditions.push(SoftCondition { category: Category::Operations, min_share: 0.5 });
    let b = next_budget(&s, &[(A, card(0.5)), (B, card(0.5))], 0.0, 0.0);
    (s, b)
}

#[test]
fn unmet_condition_lowers_standing() {
    let (mut s, b) = with_condition();
    record_spend(&mut s, A, Category::Construction, 100.0).unwrap();
    let after = close_period(&s, &b);
    assert!(after.agencies[&A].standing < s.agencies[&A].standing);
    assert_eq!(after.agencies[&B].standing, 0.55); // B had no condition: recovery
}

#[test]
fn unmet_condition_never_blocks_spend() {
    let (mut s, _) = with_condition();
    for _ in 0..10 {
        assert_eq!(record_spend(&mut s, A, Category::Construction, 1e6), Ok(()));
    }
    assert_eq!(s.agencies[&A].spend[&Category::Construction], 1e7);
}

#[test]
fn met_condition_keeps_standing() {
    let (mut s, b) = with_condition();
    record_spend(&mut s, A, Category::Operations, 60.0).unwrap();
    record_spend(&mut s, A, Category::Construction, 40.0).unwrap();
    let after = close_period(&s, &b);
    assert!(after.agencies[&A].standing >= s.agencies[&A].standing);
    assert!(after.agencies[&A].spend.is_empty());
}

#[test]
fn standing_clamped_and_feeds_next_budget() {
    let (mut s, b) = with_condition();
    record_spend(&mut s, A, Category::Construction, 1.0).unwrap();
    s.agencies.get_mut(&A).unwrap().standing = 0.0;
    assert_eq!(close_period(&s, &b).agencies[&A].standing, 0.0);
    let cards = [(A, card(0.5)), (B, card(0.5))];
    let mut hi = two();
    hi.agencies.get_mut(&A).unwrap().standing = 1.0;
    let mut lo = two();
    lo.agencies.get_mut(&A).unwrap().standing = 0.0;
    assert!(next_budget(&hi, &cards, 0.0, 0.0).per_agency[&A].amount > next_budget(&lo, &cards, 0.0, 0.0).per_agency[&A].amount);
}

#[test]
fn spend_unknown_agency_rejected() {
    let mut s = two();
    assert_eq!(record_spend(&mut s, AgencyId(9), Category::Operations, 1.0), Err(EconError::UnknownAgency(AgencyId(9))));
    assert_eq!(record_spend(&mut s, A, Category::Operations, f64::NAN), Err(EconError::BadAmount));
}

#[test]
fn greenfield_cheaper_than_developed() {
    let p = LandParams::default();
    let dev = Cell::Developed { density: 5.0, land_value: 2.0 };
    assert!(land_cost(&[Cell::Greenfield], &p) < land_cost(&[dev], &p));
}

#[test]
fn reserved_corridor_cheaper_than_later_acquisition() {
    let p = LandParams::default();
    let mut cells = vec![Cell::Greenfield; 10];
    let reserve_cost = reserve(&mut cells, &p);
    assert!(cells.iter().all(|c| *c == Cell::Reserved));
    assert_eq!(land_cost(&cells, &p), 0.0);
    let developed = vec![Cell::Developed { density: 5.0, land_value: land_value(1.0, 100.0, &p) }; 10];
    assert!(reserve_cost + land_cost(&cells, &p) < land_cost(&developed, &p));
}

#[test]
fn land_value_rises_with_accessibility() {
    let p = LandParams::default();
    assert!(land_value(1.0, 200.0, &p) > land_value(1.0, 10.0, &p));
    assert_eq!(land_value(1.0, 0.0, &p), 1.0);
}

#[test]
fn build_cost_sums_base_and_land() {
    let p = LandParams::default();
    assert_eq!(build_cost(3.0, 4.0, &[Cell::Greenfield, Cell::Reserved], &p), 12.0 + p.greenfield_rate);
}

#[test]
fn plugin_period_issues_budget() {
    let mut app = App::new();
    app.add_plugins(EconomyPlugin);
    app.world_mut().resource_mut::<PeriodClock>().period = 2;
    {
        let mut e = app.world_mut().resource_mut::<Economy>();
        e.0.add_agency(A, 0.5);
        e.0.add_agency(B, 0.5);
    }
    app.world_mut().resource_mut::<Scorecards>().0 = vec![(A, card(0.8)), (B, card(0.2))];
    app.world_mut().resource_mut::<SpendQueue>().0 =
        vec![SpendCommand { agency: A, category: Category::Operations, amount: 5.0 }, SpendCommand { agency: AgencyId(7), category: Category::Operations, amount: 1.0 }];
    app.world_mut().run_schedule(FixedUpdate);
    assert!(app.world().resource::<CurrentBudget>().0.is_none());
    assert_eq!(app.world().resource::<SpendErrors>().0.len(), 1);
    app.world_mut().run_schedule(FixedUpdate);
    let b = app.world().resource::<CurrentBudget>().0.clone().unwrap();
    assert!(b.per_agency[&A].amount > b.per_agency[&B].amount);
}
