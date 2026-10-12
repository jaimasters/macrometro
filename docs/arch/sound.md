# Sound arch
Pure-Rust synthesis -> bevy_audio custom `Decodable` (`Tone`, mono f32 buffer). No samples, no new deps (wasm size).
Flow: any crate writes `SoundEvent{kind,pitch,pan}` message -> `play_events` quantises `pitch` to A-major pentatonic (A3 root, octave clamped -1..+2), renders a decaying sine tone, spawns `AudioPlayer` (DESPAWN). Voice cap `MAX_VOICES`=8 (excess dropped). Looping 8 s root+fifth drone is the ambient bed.
`SoundSettings{muted,volume}` scales new voices and live sinks on change. Web unlock: deferred (web paused); browsers resume AudioContext on first gesture via cpal/bevy.
