# sound SPEC
Crate `sound`, `SoundPlugin`. API: `SoundEvent{kind: SoundKind{TrainArrive,StationPlaced,LineCreated}, pitch: i32, pan: f32}` (Message), `SoundSettings{muted, volume}` (Resource), `quantise`, `render_tone`, `admit`.
App wiring: `app.add_plugins(sound::SoundPlugin)` (+ `sound` dep in crates/app).
Acceptance (`cargo test -p sound`): quantise_hits_scale, voice_limit, tone_nonsilent_bounded_deterministic.
