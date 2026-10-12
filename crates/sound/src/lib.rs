//! Mini Metro-style generative soundscape. Other crates write `SoundEvent`;
//! this crate quantises to a pentatonic scale, synthesises a short tone in pure
//! Rust (no samples, no extra deps) and plays it through bevy_audio.
use bevy::audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source, Volume};
use bevy::prelude::*;
use std::sync::Arc;
use std::time::Duration;

pub const RATE: u32 = 44_100;
/// Major pentatonic offsets (semitones): never dissonant, whatever fires together.
pub const SCALE: [u8; 5] = [0, 2, 4, 7, 9];
pub const MAX_VOICES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundKind { TrainArrive, StationPlaced, LineCreated }

/// Emit from any crate: `writer.write(SoundEvent{kind, pitch: line_index, pan: 0.0})`.
/// `pitch` is any integer (e.g. line or station id); it is quantised to the scale.
#[derive(Message, Clone, Copy, Debug)]
pub struct SoundEvent { pub kind: SoundKind, pub pitch: i32, pub pan: f32 }

/// Player settings. `muted` silences new and existing sounds; `volume` 0..=1.
#[derive(Resource, Clone, Copy, Debug)]
pub struct SoundSettings { pub muted: bool, pub volume: f32 }
impl Default for SoundSettings { fn default() -> Self { Self { muted: false, volume: 0.6 } } }

/// Scale degree -> frequency (Hz), rooted at A3 = 220 Hz.
pub fn quantise(pitch: i32) -> f32 {
    let n = SCALE.len() as i32;
    let (oct, deg) = (pitch.div_euclid(n), pitch.rem_euclid(n) as usize);
    let semis = (oct.clamp(-1, 2) * 12 + SCALE[deg] as i32) as f32;
    220.0 * 2f32.powf(semis / 12.0)
}

/// Mono sine + soft 2nd harmonic with exponential decay; deterministic, |s| <= amp.
pub fn render_tone(freq: f32, secs: f32, amp: f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    (0..n).map(|i| {
        let t = i as f32 / RATE as f32;
        let attack = (t / 0.005).min(1.0);
        let env = attack * (-t * 5.0 / secs).exp();
        let w = std::f32::consts::TAU * freq * t;
        amp * env * (0.8 * w.sin() + 0.2 * (2.0 * w).sin())
    }).collect()
}

/// Admit a new voice only if fewer than MAX_VOICES are sounding.
pub fn admit(active: usize) -> bool { active < MAX_VOICES }

#[derive(Asset, TypePath, Clone)]
pub struct Tone(pub Arc<[f32]>);
pub struct ToneDecoder { s: Arc<[f32]>, i: usize }
impl Iterator for ToneDecoder {
    type Item = bevy::audio::Sample;
    fn next(&mut self) -> Option<Self::Item> { let v = self.s.get(self.i).copied(); self.i += 1; v }
}
impl Source for ToneDecoder {
    fn current_span_len(&self) -> Option<usize> { Some(self.s.len().saturating_sub(self.i)) }
    fn channels(&self) -> ChannelCount { ChannelCount::new(1).unwrap() }
    fn sample_rate(&self) -> SampleRate { SampleRate::new(RATE).unwrap() }
    fn total_duration(&self) -> Option<Duration> { Some(Duration::from_secs_f32(self.s.len() as f32 / RATE as f32)) }
}
impl Decodable for Tone {
    type Decoder = ToneDecoder;
    fn decoder(&self) -> ToneDecoder { ToneDecoder { s: self.0.clone(), i: 0 } }
}

#[derive(Component)] struct Voice;
#[derive(Component)] struct Bed;

pub struct SoundPlugin;
impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Tone>()
            .add_message::<SoundEvent>()
            .init_resource::<SoundSettings>()
            .add_systems(Startup, spawn_bed)
            .add_systems(Update, (play_events, apply_settings));
    }
}

fn gain(s: &SoundSettings) -> f32 { if s.muted { 0.0 } else { s.volume.clamp(0.0, 1.0) } }

fn spawn_bed(mut c: Commands, mut tones: ResMut<Assets<Tone>>, s: Res<SoundSettings>) {
    // Soft looping drone: root + fifth, slow 8 s swell, seamless loop (whole cycles).
    let n = 8 * RATE as usize;
    let bed: Vec<f32> = (0..n).map(|i| {
        let t = i as f32 / RATE as f32;
        let swell = 0.5 - 0.5 * (std::f32::consts::TAU * t / 8.0).cos();
        0.05 * (0.3 + 0.7 * swell) * ((std::f32::consts::TAU * 110.0 * t).sin() + 0.6 * (std::f32::consts::TAU * 165.0 * t).sin())
    }).collect();
    c.spawn((Bed, AudioPlayer(tones.add(Tone(bed.into()))), PlaybackSettings::LOOP.with_volume(Volume::Linear(gain(&s)))));
}

fn play_events(mut c: Commands, mut ev: MessageReader<SoundEvent>, mut tones: ResMut<Assets<Tone>>,
               s: Res<SoundSettings>, voices: Query<(), With<Voice>>) {
    let mut active = voices.iter().count();
    for e in ev.read() {
        if s.muted || !admit(active) { continue; }
        let (oct, secs, amp) = match e.kind {
            SoundKind::TrainArrive => (5, 0.6, 0.25),
            SoundKind::StationPlaced => (0, 1.2, 0.3),
            SoundKind::LineCreated => (-5, 2.0, 0.35),
        };
        let tone = tones.add(Tone(render_tone(quantise(e.pitch + oct), secs, amp).into()));
        // ponytail: pan ignored (mono); add spatial audio when stereo matters.
        c.spawn((Voice, AudioPlayer(tone), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain(&s)))));
        active += 1;
    }
}

fn apply_settings(s: Res<SoundSettings>, mut sinks: Query<&mut AudioSink>) {
    if !s.is_changed() { return; }
    for mut k in &mut sinks { k.set_volume(Volume::Linear(gain(&s))); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn quantise_hits_scale() {
        assert_eq!(quantise(0), 220.0);
        assert!((quantise(3) - 220.0 * 2f32.powf(7.0 / 12.0)).abs() < 1e-3);
        assert!((quantise(5) - 440.0).abs() < 1e-3);
        assert!((quantise(-5) - 110.0).abs() < 1e-3);
        assert_eq!(quantise(1000), quantise(10)); // octave clamped to +2
    }
    #[test] fn voice_limit() { assert!(admit(MAX_VOICES - 1)); assert!(!admit(MAX_VOICES)); }
    #[test] fn tone_nonsilent_bounded_deterministic() {
        let a = render_tone(440.0, 0.5, 0.3);
        assert_eq!(a.len(), RATE as usize / 2);
        assert!(a.iter().all(|s| s.abs() <= 0.3));
        assert!(a.iter().map(|s| s.abs()).fold(0.0, f32::max) > 0.1);
        assert_eq!(a, render_tone(440.0, 0.5, 0.3));
    }
}
