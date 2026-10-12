//! Pure-Rust economy core: agencies, scorecards, budgets, soft conditions, standing, land cost. No bevy.
//! Spec: docs/features/economy/SPEC.md; D-6..D-9; arch: land-acquisition.md.
use std::collections::BTreeMap;

pub use sim_core::AgencyId; // D-8: shared type owned by sim_core.

/// Spend category a soft condition can target ("spend >= X% on Y").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Construction,
    Acquisition,
    Operations,
    /// Road-adjacent earmarks / pet projects (favouritism, D-6).
    RoadAdjacent,
}

/// Six metrics (D-9), each normalised to 0..=1, higher = better (trip_time/crowding/emissions already inverted).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Scorecard {
    pub accessibility: f64,
    pub trip_time: f64,
    pub transit_share: f64,
    pub crowding: f64,
    pub emissions: f64,
    pub equity: f64,
}

impl Scorecard {
    /// Equal weights (D-9 provisional).
    pub fn performance(&self) -> f64 {
        let v = [self.accessibility, self.trip_time, self.transit_share, self.crowding, self.emissions, self.equity];
        v.iter().map(|x| x.clamp(0.0, 1.0)).sum::<f64>() / 6.0
    }
}

/// "Spend at least `min_share` of the period's spend on `category`." Missing it costs standing, never blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftCondition {
    pub category: Category,
    pub min_share: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgencyState {
    /// Political standing 0..=1.
    pub standing: f64,
    /// City-imposed conditions (earmarks from favouritism are added per budget).
    pub conditions: Vec<SoftCondition>,
    /// Spend recorded this period.
    pub spend: BTreeMap<Category, f64>,
}

impl AgencyState {
    pub fn new(standing: f64) -> Self {
        Self { standing: standing.clamp(0.0, 1.0), conditions: vec![], spend: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EconomyParams {
    /// City revenue per period at neutral performance.
    pub base_revenue: f64,
    /// Cap on the fraction of total the city may reserve for its own roads (D-7).
    pub max_road_reserve: f64,
    /// Earmark share demanded on RoadAdjacent per unit car_share (favouritism, D-6).
    pub earmark_k: f64,
    /// Standing lost per unit of total condition shortfall.
    pub standing_penalty_k: f64,
    /// Standing regained per period when every condition is met.
    pub standing_recovery: f64,
    /// Floor so a zero-performing agency keeps a nonzero weight.
    pub eps: f64,
}

impl Default for EconomyParams {
    fn default() -> Self {
        Self { base_revenue: 1000.0, max_road_reserve: 0.4, earmark_k: 0.3, standing_penalty_k: 0.5, standing_recovery: 0.05, eps: 0.05 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EconomyState {
    pub params: EconomyParams,
    pub agencies: BTreeMap<AgencyId, AgencyState>,
}

impl EconomyState {
    pub fn new(params: EconomyParams) -> Self {
        Self { params, agencies: BTreeMap::new() }
    }
    pub fn add_agency(&mut self, id: AgencyId, standing: f64) {
        self.agencies.insert(id, AgencyState::new(standing));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgencyBudget {
    pub amount: f64,
    pub conditions: Vec<SoftCondition>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Budget {
    pub total: f64,
    /// City's own road-building reserve; competes with agency allocation (D-7).
    pub road_reserve: f64,
    pub per_agency: BTreeMap<AgencyId, AgencyBudget>,
}

/// Raise or add the earmark condition (never lowers an existing one).
fn with_earmark(mut c: Vec<SoftCondition>, share: f64) -> Vec<SoftCondition> {
    if share <= 0.0 {
        return c;
    }
    match c.iter_mut().find(|x| x.category == Category::RoadAdjacent) {
        Some(x) => x.min_share = x.min_share.max(share),
        None => c.push(SoftCondition { category: Category::RoadAdjacent, min_share: share }),
    }
    c
}

/// Deterministic budget for the next period (D-6, D-7, D-8). Pure: BTreeMap iteration, fixed op order.
/// total = base*(0.5 + 0.5*mean_perf + 0.5*mean_standing); road_reserve = min(road_demand, total*max_road_reserve)
/// where `road_demand` is growth::AgencyOutput::road_reserve (cost of pending city road plans);
/// the rest is split by (perf+eps)^(1+car_share) * (0.5+standing). Car-dependent cities (car_share up)
/// concentrate funds on the favoured performer and attach RoadAdjacent earmarks.
pub fn next_budget(s: &EconomyState, cards: &[(AgencyId, Scorecard)], car_share: f64, road_demand: f64) -> Budget {
    let p = &s.params;
    let car = car_share.clamp(0.0, 1.0);
    let perf = |id: &AgencyId| cards.iter().find(|(a, _)| a == id).map_or(0.0, |(_, c)| c.performance());
    let n = s.agencies.len().max(1) as f64;
    let mean_perf = s.agencies.keys().map(perf).sum::<f64>() / n;
    let mean_standing = s.agencies.values().map(|a| a.standing).sum::<f64>() / n;
    let total = p.base_revenue * (0.5 + 0.5 * mean_perf + 0.5 * mean_standing);
    let road_reserve = road_demand.max(0.0).min(total * p.max_road_reserve);
    let pool = total - road_reserve;
    let weights: Vec<f64> =
        s.agencies.iter().map(|(id, a)| (perf(id) + p.eps).powf(1.0 + car) * (0.5 + a.standing)).collect();
    let wsum: f64 = weights.iter().sum();
    let per_agency = s
        .agencies
        .iter()
        .zip(&weights)
        .map(|((id, a), w)| {
            let conditions = with_earmark(a.conditions.clone(), p.earmark_k * car);
            (*id, AgencyBudget { amount: if wsum > 0.0 { pool * w / wsum } else { 0.0 }, conditions })
        })
        .collect();
    Budget { total, road_reserve, per_agency }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EconError {
    UnknownAgency(AgencyId),
    BadAmount,
}

/// Record spend. Never blocked by conditions (D-6): only an unknown agency or a non-finite/negative amount is refused.
pub fn record_spend(s: &mut EconomyState, agency: AgencyId, category: Category, amount: f64) -> Result<(), EconError> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(EconError::BadAmount);
    }
    let a = s.agencies.get_mut(&agency).ok_or(EconError::UnknownAgency(agency))?;
    *a.spend.entry(category).or_insert(0.0) += amount;
    Ok(())
}

/// Sum over conditions of how far the spend share falls short. 0 when met or when nothing was spent.
pub fn shortfall(conditions: &[SoftCondition], spend: &BTreeMap<Category, f64>) -> f64 {
    let total: f64 = spend.values().sum();
    if total <= 0.0 {
        return 0.0;
    }
    conditions.iter().map(|c| (c.min_share - spend.get(&c.category).copied().unwrap_or(0.0) / total).max(0.0)).sum()
}

/// End of period: unmet conditions of the issued budget lower standing; all met recovers a little; spend resets.
pub fn close_period(s: &EconomyState, issued: &Budget) -> EconomyState {
    let mut out = s.clone();
    for (id, a) in out.agencies.iter_mut() {
        let conds = issued.per_agency.get(id).map_or(&[][..], |b| &b.conditions[..]);
        let miss = shortfall(conds, &a.spend);
        let delta = if miss > 0.0 { -s.params.standing_penalty_k * miss } else { s.params.standing_recovery };
        a.standing = (a.standing + delta).clamp(0.0, 1.0);
        a.spend.clear();
    }
    out
}

/// One cell crossed by a track corridor (land-acquisition.md).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cell {
    Greenfield,
    Developed { density: f64, land_value: f64 },
    /// Cheaply reserved earlier: no further land cost, grows nothing (growth honours this).
    Reserved,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LandParams {
    pub greenfield_rate: f64,
    pub reserve_rate: f64,
    pub k_density: f64,
    /// Extra fraction on compulsory acquisition.
    pub disruption: f64,
    /// Accessibility half-saturation for land value.
    pub k_access: f64,
}

impl Default for LandParams {
    fn default() -> Self {
        Self { greenfield_rate: 1.0, reserve_rate: 0.2, k_density: 1.0, disruption: 0.5, k_access: 50.0 }
    }
}

/// Land value rises with accessibility A_i: base*(1 + A/(A+k)).
pub fn land_value(base: f64, accessibility: f64, p: &LandParams) -> f64 {
    base * (1.0 + accessibility / (accessibility + p.k_access))
}

pub fn land_cost(cells: &[Cell], p: &LandParams) -> f64 {
    cells
        .iter()
        .map(|c| match c {
            Cell::Greenfield => p.greenfield_rate,
            Cell::Developed { density, land_value } => p.k_density * density * land_value * (1.0 + p.disruption),
            Cell::Reserved => 0.0,
        })
        .sum()
}

/// base(track, length) + land(cells crossed).
pub fn build_cost(base_per_len: f64, len: f64, cells: &[Cell], p: &LandParams) -> f64 {
    base_per_len * len + land_cost(cells, p)
}

/// Reserve greenfield cells (rewards foresight); returns the cost. Developed/already-reserved cells untouched, free.
pub fn reserve(cells: &mut [Cell], p: &LandParams) -> f64 {
    let mut cost = 0.0;
    for c in cells.iter_mut() {
        if *c == Cell::Greenfield {
            *c = Cell::Reserved;
            cost += p.reserve_rate;
        }
    }
    cost
}
