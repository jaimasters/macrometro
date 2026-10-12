//! `cargo run -p save --bin gen-scenarios -- --seed N [--out DIR]` writes DIR/{fresh,mid_growth,sprawl_crisis}.ron.
use save::{scenario::{generate, Scenario}, to_ron};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let seed: u64 = get("--seed").map(|s| s.parse().expect("--seed N")).unwrap_or(1);
    let out = get("--out").unwrap_or_else(|| "saves".into());
    std::fs::create_dir_all(&out).expect("create out dir");
    for s in Scenario::ALL {
        let path = format!("{out}/{}.ron", s.name());
        std::fs::write(&path, to_ron(&generate(s, seed))).expect("write save");
        println!("wrote {path}");
    }
}
