use super::*;
use std::time::Duration;

#[test]
fn clock_accumulates_fractional_ticks() {
    let mut c = FixedClock::new(10);
    assert_eq!(c.advance(Duration::from_millis(60), 10), 0);
    assert_eq!(c.advance(Duration::from_millis(60), 10), 1);
    assert_eq!(c.advance(Duration::from_millis(80), 10), 1);
}

#[test]
fn clock_caps_catchup() {
    let mut c = FixedClock::new(10);
    assert_eq!(c.advance(Duration::from_secs(10), 5), 5);
    assert_eq!(c.advance(Duration::from_millis(0), 5), 0);
}

#[test]
fn clock_alpha_in_range() {
    let mut c = FixedClock::new(10);
    c.advance(Duration::from_millis(50), 10);
    let a = c.alpha();
    assert!((a - 0.5).abs() < 1e-3, "{a}");
    c.advance(Duration::from_millis(50), 10);
    assert!((0.0..1.0).contains(&c.alpha()));
}

#[test]
fn command_queue_fifo_applied_at_tick_boundary() {
    let (mut sim, tx, out) = Sim::new(CoreSim::new(7));
    tx.send(Command::Custom(1, 5).by(AgencyId(0))).unwrap();
    tx.send(Command::Custom(2, 9).by(AgencyId(0))).unwrap();
    assert_eq!(sim.state.custom_sum, 0, "not applied before tick boundary");
    sim.tick_once();
    assert_eq!(sim.state.custom_sum, (5 ^ 1) + (9 ^ 2));
    assert_eq!(out.latest().tick, 1);
}

#[test]
fn pause_resume_speed_commands() {
    let (mut sim, tx, out) = Sim::new(CoreSim::new(7));
    tx.send(Command::Pause.by(AgencyId(0))).unwrap();
    sim.tick_once();
    assert_eq!(out.latest().tick, 0);
    tx.send(Command::Resume.by(AgencyId(0))).unwrap();
    tx.send(Command::SetSpeed(3).by(AgencyId(0))).unwrap();
    sim.tick_once();
    assert_eq!(out.latest().tick, 3);
}

#[test]
fn handoff_returns_newest() {
    let h = Handoff::new(1u32);
    h.publish(Arc::new(2));
    h.publish(Arc::new(3));
    assert_eq!(*h.latest(), 3);
}

#[test]
fn handoff_latest_never_blocks_during_publish() {
    let h = Arc::new(Handoff::new(0u64));
    let w = h.clone();
    let t = std::thread::spawn(move || {
        for i in 0..20_000 {
            w.publish(Arc::new(i));
        }
    });
    let start = std::time::Instant::now();
    let mut last = 0;
    while !t.is_finished() {
        let v = *h.latest();
        assert!(v >= last, "monotonic");
        last = v;
    }
    t.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn pump_budget_limits_ticks() {
    let (mut sim, _tx, out) = Sim::new(CoreSim::new(1));
    let mut c = FixedClock::new(10);
    assert_eq!(sim.pump(&mut c, Duration::from_secs(5), 3), 3);
    assert_eq!(out.latest().tick, 3);
}

#[test]
fn same_seed_same_hash() {
    let (mut a, tx_a, out_a) = Sim::new(CoreSim::new(42));
    let (mut b, tx_b, out_b) = Sim::new(CoreSim::new(42));
    for i in 0..100u32 {
        for tx in [&tx_a, &tx_b] {
            match i {
                10 => tx.send(Command::Custom(1, 5).by(AgencyId(0))).unwrap(),
                20 => tx.send(Command::SetSpeed(2).by(AgencyId(0))).unwrap(),
                50 => tx.send(Command::Pause.by(AgencyId(0))).unwrap(),
                55 => tx.send(Command::Resume.by(AgencyId(0))).unwrap(),
                _ => {}
            }
        }
        a.tick_once();
        b.tick_once();
    }
    assert_eq!(a.state.state_hash(), b.state.state_hash());
    assert_eq!(out_a.latest().hash, a.state.state_hash());
    assert_eq!(out_b.latest().hash, b.state.state_hash());
    assert_eq!(out_a.latest().tick, out_b.latest().tick);
}

#[test]
fn different_seed_different_hash() {
    let (mut a, _, _) = Sim::new(CoreSim::new(1));
    let (mut b, _, _) = Sim::new(CoreSim::new(2));
    for _ in 0..100 {
        a.tick_once();
        b.tick_once();
    }
    assert_ne!(a.state.state_hash(), b.state.state_hash());
}

#[test]
fn thread_matches_direct_stepping() {
    let (sim, _tx, out) = Sim::new(CoreSim::new(9));
    let mut th = SimThread::spawn(sim, 200);
    let start = std::time::Instant::now();
    while out.latest().tick < 20 {
        assert!(start.elapsed() < Duration::from_secs(5), "sim thread timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    th.stop();
    let s = out.latest();
    let mut direct = CoreSim::new(9);
    for i in 0..s.tick {
        direct.step(i);
    }
    assert_eq!(direct.state_hash(), s.hash);
}

#[test]
fn thread_stop_joins() {
    let (sim, _tx, out) = Sim::new(CoreSim::new(9));
    let mut th = SimThread::spawn(sim, 200);
    std::thread::sleep(Duration::from_millis(50));
    th.stop();
    let t1 = out.latest().tick;
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(out.latest().tick, t1, "thread kept advancing after stop");
    assert!(t1 > 0);
}

#[test]
fn identical_command_streams_identical_hashes() {
    let run = |script: &[(u64, u32, Command)]| {
        let (mut sim, tx, _out) = Sim::new(CoreSim::new(5));
        for t in 0..60u64 {
            for (at, ag, c) in script {
                if *at == t {
                    tx.send(c.by(AgencyId(*ag))).unwrap();
                }
            }
            sim.tick_once();
        }
        sim.state.state_hash()
    };
    let s = [(3, 1, Command::Custom(7, 11)), (3, 2, Command::Custom(8, 12)), (30, 1, Command::SetSpeed(2))];
    assert_eq!(run(&s), run(&s));
    // same commands, different issuing agency => different state
    let s2 = [(3, 2, Command::Custom(7, 11)), (3, 2, Command::Custom(8, 12)), (30, 1, Command::SetSpeed(2))];
    assert_ne!(run(&s), run(&s2));
}
