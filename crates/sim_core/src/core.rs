//! Pure-Rust sim core: fixed clock, command queue, snapshot handoff, sim thread. No bevy.
//! Spec: docs/features/sim_core/SPEC.md.
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Fixed sim tick rate (Hz). Decoupled from render; see docs/arch/sim-render-decoupling.md.
pub const SIM_HZ: u32 = 10;

/// FNV-1a over u64 words; the determinism hash primitive.
pub fn fnv1a(words: &[u64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for w in words {
        for b in w.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Seeded xorshift64* RNG. The only RNG sims may use (determinism rule).
#[derive(Debug, Clone)]
pub struct SeededRng(u64);

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    /// Uniform in [0,1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Accumulator clock: real elapsed time in, whole sim ticks out.
#[derive(Debug, Clone)]
pub struct FixedClock {
    period: Duration,
    acc: Duration,
}

impl FixedClock {
    pub fn new(hz: u32) -> Self {
        Self { period: Duration::from_secs_f64(1.0 / hz.max(1) as f64), acc: Duration::ZERO }
    }
    pub fn period(&self) -> Duration {
        self.period
    }
    /// Add `dt`; return ticks due, at most `max_ticks`. Time beyond the cap is dropped (no spiral of death).
    pub fn advance(&mut self, dt: Duration, max_ticks: u32) -> u32 {
        self.acc += dt;
        let due = (self.acc.as_nanos() / self.period.as_nanos()) as u32;
        let n = due.min(max_ticks);
        if due > max_ticks {
            self.acc = Duration::ZERO;
        } else {
            self.acc -= self.period * n;
        }
        n
    }
    /// Fraction of the current period elapsed since the last tick, in [0,1).
    pub fn alpha(&self) -> f32 {
        (self.acc.as_secs_f64() / self.period.as_secs_f64()).clamp(0.0, 0.999_999) as f32
    }
}

/// Player/system input, applied at the next tick boundary in send order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Pause,
    Resume,
    /// Sim steps per tick (0 behaves as paused).
    SetSpeed(u8),
    /// Extension point for other crates: (kind, arg).
    Custom(u32, i64),
}

/// Agency (player/AI/city) issuing a command. Shared type; economy imports it (D-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AgencyId(pub u32);

/// A command tagged with its issuing agency: the only thing that enters the sim (input-driven, D-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgencyCommand {
    pub agency: AgencyId,
    pub cmd: Command,
}

impl Command {
    pub fn by(self, agency: AgencyId) -> AgencyCommand {
        AgencyCommand { agency, cmd: self }
    }
}

pub type CommandSender = Sender<AgencyCommand>;
pub type CommandQueue = Receiver<AgencyCommand>;

pub fn command_channel() -> (CommandSender, CommandQueue) {
    mpsc::channel()
}

/// Latest-value handoff. The lock is held only to swap/clone an `Arc`, never during sim work.
#[derive(Debug)]
pub struct Handoff<T>(RwLock<Arc<T>>);

impl<T> Handoff<T> {
    pub fn new(initial: T) -> Self {
        Self(RwLock::new(Arc::new(initial)))
    }
    pub fn publish(&self, v: Arc<T>) {
        *self.0.write().unwrap() = v;
    }
    pub fn latest(&self) -> Arc<T> {
        self.0.read().unwrap().clone()
    }
}

/// A simulation advanced by `Sim`. Implemented here by `CoreSim`; growth etc. plug in via app.
pub trait Simulation: Send + 'static {
    type Snapshot: Send + Sync + 'static;
    /// Custom commands (Pause/Resume/SetSpeed are handled by `Sim`).
    fn apply(&mut self, cmd: &AgencyCommand);
    /// Advance one step. `tick` is the index of the step being taken.
    fn step(&mut self, tick: u64);
    fn snapshot(&self, tick: u64) -> Self::Snapshot;
    fn state_hash(&self) -> u64;
}

/// Drives a `Simulation`: drain commands -> step -> publish snapshot, once per tick.
pub struct Sim<S: Simulation> {
    pub state: S,
    pub tick: u64,
    pub paused: bool,
    pub speed: u8,
    rx: CommandQueue,
    out: Arc<Handoff<S::Snapshot>>,
}

impl<S: Simulation> Sim<S> {
    /// Returns the sim plus the command sender and snapshot handoff for the other side.
    pub fn new(state: S) -> (Self, CommandSender, Arc<Handoff<S::Snapshot>>) {
        let (tx, rx) = command_channel();
        let out = Arc::new(Handoff::new(state.snapshot(0)));
        (Self { state, tick: 0, paused: false, speed: 1, rx, out: out.clone() }, tx, out)
    }

    pub fn tick_once(&mut self) {
        while let Ok(ac) = self.rx.try_recv() {
            match ac.cmd {
                Command::Pause => self.paused = true,
                Command::Resume => self.paused = false,
                Command::SetSpeed(s) => self.speed = s,
                _ => self.state.apply(&ac),
            }
        }
        if !self.paused {
            for _ in 0..self.speed {
                self.state.step(self.tick);
                self.tick += 1;
            }
        }
        self.out.publish(Arc::new(self.state.snapshot(self.tick)));
    }

    /// Time-sliced driver (wasm main thread / tests): run due ticks, at most `budget_ticks`.
    pub fn pump(&mut self, clock: &mut FixedClock, dt: Duration, budget_ticks: u32) -> u32 {
        let n = clock.advance(dt, budget_ticks);
        for _ in 0..n {
            self.tick_once();
        }
        n
    }
}

/// Worker thread running a `Sim` at `hz`. Not available on wasm (use `Sim::pump`).
#[cfg(not(target_arch = "wasm32"))]
pub struct SimThread {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SimThread {
    pub fn spawn<S: Simulation>(mut sim: Sim<S>, hz: u32) -> Self {
        use std::sync::atomic::Ordering;
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut clock = FixedClock::new(hz);
            let mut last = std::time::Instant::now();
            while !flag.load(Ordering::Relaxed) {
                let now = std::time::Instant::now();
                sim.pump(&mut clock, now - last, 5);
                last = now;
                std::thread::sleep(clock.period() / 4);
            }
        });
        Self { stop, handle: Some(handle) }
    }
    pub fn stop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SimThread {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Reference simulation: seeded RNG walk. Used by tests and the plugin until real systems exist.
#[derive(Debug, Clone)]
pub struct CoreSim {
    rng: SeededRng,
    pub acc: u64,
    pub custom_sum: i64,
}

impl CoreSim {
    pub fn new(seed: u64) -> Self {
        Self { rng: SeededRng::new(seed), acc: 0, custom_sum: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSnapshot {
    pub tick: u64,
    pub hash: u64,
}

impl Simulation for CoreSim {
    type Snapshot = CoreSnapshot;
    fn apply(&mut self, ac: &AgencyCommand) {
        if let Command::Custom(k, v) = ac.cmd {
            // agency folded in so who-issued-it affects the hash
            self.custom_sum = self.custom_sum.wrapping_add((v ^ k as i64).wrapping_mul(ac.agency.0 as i64 + 1));
        }
    }
    fn step(&mut self, _tick: u64) {
        self.acc = self.acc.wrapping_add(self.rng.next_u64());
    }
    fn snapshot(&self, tick: u64) -> CoreSnapshot {
        CoreSnapshot { tick, hash: self.state_hash() }
    }
    fn state_hash(&self) -> u64 {
        fnv1a(&[self.acc, self.custom_sum as u64])
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
