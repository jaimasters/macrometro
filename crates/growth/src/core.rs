//! Pure-Rust growth core: zones, cost matrix, accessibility, gravity OD flows, growth deltas. No bevy.
//! Spec: docs/features/growth/SPEC.md; arch: aggregation.md, growth-drivers.md.

/// Coarse zone identifier for aggregated (out-of-focus) simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub id: ZoneId,
    pub pop: f64,
    pub jobs: f64,
    /// Max population the zone's land supports.
    pub capacity: f64,
    /// Load factor of lines serving the zone (>1 = overcrowded).
    pub crowding: f64,
}

/// Zones in ascending ZoneId order; matrix index i == zones.0[i].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Zones(pub Vec<Zone>);

impl Zones {
    pub fn new(mut v: Vec<Zone>) -> Self {
        v.sort_by_key(|z| z.id);
        Self(v)
    }
}

/// Generalised cost C[i][j]; `f64::INFINITY` = unreachable. Dense (ponytail: 10k zones = 800MB; sparse rows later).
#[derive(Debug, Clone, PartialEq)]
pub struct CostMatrix {
    pub n: usize,
    c: Vec<f64>,
}

impl CostMatrix {
    /// All pairs unreachable except the diagonal (cost 0).
    pub fn new(n: usize) -> Self {
        Self::from_fn(n, |i, j| if i == j { 0.0 } else { f64::INFINITY })
    }
    pub fn from_fn(n: usize, f: impl Fn(usize, usize) -> f64) -> Self {
        let mut c = Vec::with_capacity(n * n);
        for i in 0..n {
            for j in 0..n {
                c.push(f(i, j));
            }
        }
        Self { n, c }
    }
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.c[i * self.n + j]
    }
    pub fn set(&mut self, i: usize, j: usize, v: f64) {
        self.c[i * self.n + j] = v;
    }
}

/// exp(-beta*c); 0 for unreachable.
fn attract(beta: f64, c: f64) -> f64 {
    if c.is_finite() { (-beta * c).exp() } else { 0.0 }
}

/// A_i = sum_j jobs_j * exp(-beta*C_ij).
pub fn accessibility(zones: &Zones, c: &CostMatrix, beta: f64) -> Vec<f64> {
    assert_eq!(zones.0.len(), c.n);
    (0..c.n)
        .map(|i| (0..c.n).map(|j| zones.0[j].jobs * attract(beta, c.get(i, j))).sum())
        .collect()
}

/// OD flow matrix F[i][j] trips/tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Flows {
    pub n: usize,
    f: Vec<f64>,
}

impl Flows {
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.f[i * self.n + j]
    }
    pub fn row_sum(&self, i: usize) -> f64 {
        self.f[i * self.n..(i + 1) * self.n].iter().sum()
    }
    pub fn total(&self) -> f64 {
        self.f.iter().sum()
    }
}

/// Singly-constrained gravity: origin i emits pop_i*trip_rate, split over j by jobs_j*exp(-beta*C_ij).
/// An origin with no reachable job emits nothing.
pub fn gravity_flows(zones: &Zones, c: &CostMatrix, beta: f64, trip_rate: f64) -> Flows {
    let n = c.n;
    assert_eq!(zones.0.len(), n);
    let mut f = vec![0.0; n * n];
    for i in 0..n {
        let w: Vec<f64> = (0..n).map(|j| zones.0[j].jobs * attract(beta, c.get(i, j))).collect();
        let tot: f64 = w.iter().sum();
        if tot > 0.0 {
            let emit = zones.0[i].pop * trip_rate;
            for j in 0..n {
                f[i * n + j] = emit * w[j] / tot;
            }
        }
    }
    Flows { n, f }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrowthParams {
    pub beta: f64,
    pub trip_rate: f64,
    /// Max fractional growth per step.
    pub rate: f64,
    /// Accessibility half-saturation: A/(A+k).
    pub k: f64,
    /// Added to pop so empty zones can start growing.
    pub seed_pop: f64,
    /// New jobs per new resident.
    pub jobs_ratio: f64,
}

impl Default for GrowthParams {
    fn default() -> Self {
        Self { beta: 0.1, trip_rate: 0.5, rate: 0.02, k: 50.0, seed_pop: 10.0, jobs_ratio: 0.4 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneDelta {
    pub id: ZoneId,
    pub dpop: f64,
    pub djobs: f64,
}

/// Per-zone growth, independent given A (parallel-safe). growth-drivers.md.
pub fn growth_deltas(zones: &Zones, acc: &[f64], p: &GrowthParams) -> Vec<ZoneDelta> {
    zones
        .0
        .iter()
        .zip(acc)
        .map(|(z, &a)| {
            let land = (1.0 - z.pop / z.capacity).max(0.0);
            let crowd = 1.0 / (1.0 + (z.crowding - 1.0).max(0.0));
            let g = p.rate * (a / (a + p.k)) * land * crowd;
            let dpop = g * (z.pop + p.seed_pop);
            ZoneDelta { id: z.id, dpop, djobs: dpop * p.jobs_ratio }
        })
        .collect()
}

/// Merge deltas in ascending ZoneId order (deterministic regardless of producer order); pop clamped to capacity.
pub fn apply_deltas(zones: &mut Zones, deltas: &[ZoneDelta]) {
    let mut d: Vec<&ZoneDelta> = deltas.iter().collect();
    d.sort_by_key(|d| d.id);
    for d in d {
        if let Ok(i) = zones.0.binary_search_by_key(&d.id, |z| z.id) {
            let z = &mut zones.0[i];
            z.pop = (z.pop + d.dpop).clamp(0.0, z.capacity);
            z.jobs = (z.jobs + d.djobs).max(0.0);
        }
    }
}

/// FNV-1a over f64 bit patterns (growth does not depend on sim_core).
pub fn hash_f64s(vals: impl Iterator<Item = f64>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in vals {
        for b in v.to_bits().to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

pub struct GrowthStep {
    pub accessibility: Vec<f64>,
    pub flows: Flows,
}

/// Pure growth state; the plugin and (later) a sim_core Simulation wrap it.
#[derive(Debug, Clone, PartialEq)]
pub struct GrowthSim {
    pub zones: Zones,
    pub params: GrowthParams,
}

impl GrowthSim {
    pub fn new(zones: Zones, params: GrowthParams) -> Self {
        Self { zones, params }
    }
    /// One tick given the current cost matrix: accessibility -> flows -> growth.
    pub fn step(&mut self, c: &CostMatrix) -> GrowthStep {
        let accessibility = accessibility(&self.zones, c, self.params.beta);
        let flows = gravity_flows(&self.zones, c, self.params.beta, self.params.trip_rate);
        let deltas = growth_deltas(&self.zones, &accessibility, &self.params);
        apply_deltas(&mut self.zones, &deltas);
        GrowthStep { accessibility, flows }
    }
    pub fn state_hash(&self) -> u64 {
        hash_f64s(self.zones.0.iter().flat_map(|z| [z.pop, z.jobs]))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_gravity.rs"]
mod tests_gravity;
