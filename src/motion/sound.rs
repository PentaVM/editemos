//! Sound for motion graphics, made from the scene's own cues. A scene declares
//! `Motion.sfx(at, 'whoosh')` (or `sfx:` on a tween, or `<Sfx>` in a
//! composition); the renderer synthesises every cue here, sample-exact on the
//! frame grid, and muxes the result into the render. No stock audio, no
//! licences, and the same scene always produces the same waveform.

use serde::Deserialize;
use std::f32::consts::PI;
use std::path::Path;

pub const RATE: u32 = 48_000;

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Cue {
    /// Seconds from the start of the scene. For a riser this is where it lands.
    pub at: f64,
    pub kind: String,
    #[serde(default = "one")]
    pub gain: f32,
    #[serde(default = "one")]
    pub pitch: f32,
    /// -1 left .. 1 right
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub dur: Option<f32>,
    #[serde(default)]
    pub seed: u32,
    /// `kind: "file"`: an audio file next to the scene, mixed by ffmpeg.
    #[serde(default)]
    pub src: Option<String>,
    /// Seconds to skip at the start of `src`.
    #[serde(default)]
    pub trim: Option<f64>,
}

fn one() -> f32 {
    1.0
}

/// Seeded noise: the same cue always sounds the same.
struct Noise(u32);

impl Noise {
    fn new(seed: u32) -> Self {
        Noise(seed.wrapping_mul(2_654_435_761).max(1))
    }
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Chamberlin state-variable filter; `band` and `low` outputs.
struct Svf {
    low: f32,
    band: f32,
}

impl Svf {
    fn new() -> Self {
        Svf { low: 0.0, band: 0.0 }
    }
    fn run(&mut self, x: f32, cutoff: f32, q: f32) -> (f32, f32) {
        let f = (2.0 * (PI * cutoff.min(RATE as f32 / 6.5) / RATE as f32).sin()).min(1.2);
        let high = x - self.low - q * self.band;
        self.band += f * high;
        self.low += f * self.band;
        (self.band, self.low)
    }
}

/// Default length of each sound, in seconds.
pub fn default_dur(kind: &str) -> f32 {
    match kind {
        "click" => 0.06,
        "tick" => 0.035,
        "pop" => 0.14,
        "whoosh" => 0.45,
        "thud" => 0.55,
        "boom" => 1.6,
        "riser" => 1.2,
        "hit" => 0.45,
        "type" => 0.05,
        "glitch" => 0.28,
        "shimmer" => 1.3,
        _ => 0.2,
    }
}

/// Mono samples of one cue, starting at its first sample.
pub fn voice(c: &Cue) -> Vec<f32> {
    let dur = c.dur.unwrap_or_else(|| default_dur(&c.kind)).clamp(0.01, 20.0);
    let n = (dur * RATE as f32) as usize;
    let sr = RATE as f32;
    let p = c.pitch.clamp(0.25, 4.0);
    let mut rnd = Noise::new(c.seed.wrapping_add(c.kind.len() as u32 * 977));
    let mut f1 = Svf::new();
    let mut f2 = Svf::new();
    let mut phase = 0f32;
    let mut phase2 = 0f32;
    let mut held = 0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / sr;
        let u = t / dur; // 0..1 through the sound
        let s = match c.kind.as_str() {
            // a tactile switch: a bright decaying sine on a 2 ms transient
            "click" => {
                phase += 2.0 * PI * 1800.0 * p / sr;
                let tr = if t < 0.002 { rnd.next() * (1.0 - t / 0.002) * 0.5 } else { 0.0 };
                (phase.sin() * (-t * 90.0).exp()) * 0.55 + tr
            }
            "tick" => {
                phase += 2.0 * PI * 3200.0 * p / sr;
                phase.sin() * (-t * 220.0).exp() * 0.45
            }
            // a bubble: pitch falls fast as it decays
            "pop" => {
                let f = (260.0 + 640.0 * (-t * 38.0).exp()) * p;
                phase += 2.0 * PI * f / sr;
                let att = (t / 0.002).min(1.0);
                phase.sin() * att * (-t * 32.0).exp() * 0.7
            }
            // air moving past: band-passed noise sweeping up then down, bell-shaped
            "whoosh" => {
                let sweep = if u < 0.6 { u / 0.6 } else { 1.0 - (u - 0.6) / 0.4 * 0.6 };
                let cutoff = 350.0 * (9.0f32).powf(sweep) * p;
                let (band, _) = f1.run(rnd.next(), cutoff, 0.7);
                let env = (PI * u).sin().powf(1.6);
                band * env * 1.1
            }
            // a body landing: low sine with a pitch drop and a soft click on top
            "thud" | "boom" => {
                let (hi, lo, k) = if c.kind == "thud" { (95.0, 42.0, 9.0) } else { (70.0, 28.0, 3.2) };
                let f = (lo + (hi - lo) * (-t * 14.0).exp()) * p;
                phase += 2.0 * PI * f / sr;
                let body = phase.sin() * (-t * k).exp();
                let (_, rumble) = f1.run(rnd.next(), 180.0 * p, 1.0);
                let tr = if t < 0.004 { rnd.next() * (1.0 - t / 0.004) * 0.35 } else { 0.0 };
                let weight = if c.kind == "boom" { 0.9 } else { 0.3 };
                body * 0.9 + rumble * (-t * k * 0.8).exp() * weight + tr
            }
            // tension into a cut: noise and a buzzy tone both climbing, swelling to the end
            "riser" => {
                let cutoff = 300.0 * (20.0f32).powf(u) * p;
                let (band, _) = f1.run(rnd.next(), cutoff, 0.5);
                let f = 110.0 * (4.0f32).powf(u) * p;
                phase += 2.0 * PI * f / sr;
                phase2 += 2.0 * PI * f * 1.007 / sr;
                let tone = (phase.sin() + 0.5 * (2.0 * phase2).sin() + 0.25 * (3.0 * phase).sin()) * 0.25;
                let env = u * u;
                let tail = if u > 0.985 { (1.0 - u) / 0.015 } else { 1.0 };
                (band * 0.8 + tone) * env * tail
            }
            // an impact: noise burst, a punchy low and a short mid ring
            "hit" => {
                phase += 2.0 * PI * 120.0 * p / sr;
                phase2 += 2.0 * PI * 620.0 * p / sr;
                let burst = if t < 0.03 { rnd.next() * (1.0 - t / 0.03) } else { 0.0 };
                burst * 0.6 + phase.sin() * (-t * 11.0).exp() * 0.8 + phase2.sin() * (-t * 45.0).exp() * 0.25
            }
            // a key press: high-passed noise tick plus a small resonance, varied per seed
            "type" => {
                let x = rnd.next();
                let (band, low) = f1.run(x, 2600.0 * p * (0.9 + 0.2 * ((c.seed % 7) as f32 / 7.0)), 0.3);
                let hp = x - low;
                (hp * 0.35 + band * 0.6) * (-t * 160.0).exp()
            }
            // digital damage: sample-and-hold bursts switching on and off
            "glitch" => {
                let slot = (u * 9.0) as u32;
                let on = (slot.wrapping_mul(2_654_435_761) ^ c.seed) % 3 != 0;
                if i % 96 == 0 {
                    held = rnd.next();
                }
                let (band, _) = f2.run(held, 1800.0 * p, 0.4);
                if on {
                    (held * 0.4 + band * 0.5) * (1.0 - u * 0.5)
                } else {
                    0.0
                }
            }
            // a sparkle: a few detuned partials ringing out
            "shimmer" => {
                let mut v = 0.0;
                for (k, ratio) in [1.0f32, 1.5, 2.0, 3.0, 4.01].iter().enumerate() {
                    let f = 880.0 * p * ratio * (1.0 + 0.002 * k as f32);
                    v += (2.0 * PI * f * t + k as f32).sin() / (k as f32 + 1.5);
                }
                let att = (t / 0.02).min(1.0);
                v * att * (-t * 3.0).exp() * 0.35
            }
            _ => 0.0,
        };
        out.push(s);
    }
    // never click at the end of a voice
    let fade = (RATE as usize / 400).min(out.len());
    let len = out.len();
    for k in 0..fade {
        out[len - 1 - k] *= k as f32 / fade as f32;
    }
    out
}

/// Synthesised cues mixed to stereo. Files (`kind: "file"`) are skipped: ffmpeg
/// mixes those in. Returns interleaved L/R samples covering `duration` seconds.
pub fn mix(cues: &[Cue], duration: f64) -> Vec<f32> {
    let frames = (duration * RATE as f64).ceil() as usize;
    let mut buf = vec![0f32; frames * 2];
    for c in cues.iter().filter(|c| c.kind != "file") {
        let v = voice(c);
        // a riser ends on its time; everything else starts on it
        let start_s = if c.kind == "riser" { c.at - v.len() as f64 / RATE as f64 } else { c.at };
        let start = (start_s * RATE as f64).round() as i64;
        // equal-power pan: centre is -3 dB on each side
        let angle = (c.pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
        let (gl, gr) = (angle.cos() * c.gain, angle.sin() * c.gain);
        for (k, s) in v.iter().enumerate() {
            let i = start + k as i64;
            if i < 0 || i as usize >= frames {
                continue;
            }
            buf[i as usize * 2] += s * gl;
            buf[i as usize * 2 + 1] += s * gr;
        }
    }
    // keep peaks under -1 dBFS without squashing quiet scenes
    let peak = buf.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak > 0.89 {
        let g = 0.89 / peak;
        buf.iter_mut().for_each(|s| *s *= g);
    }
    buf
}

/// 16-bit PCM WAV, stereo.
pub fn write_wav(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut b: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    std::fs::write(path, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(kind: &str, at: f64) -> Cue {
        Cue { at, kind: kind.into(), gain: 1.0, pitch: 1.0, seed: 3, ..Default::default() }
    }

    fn first_sound(buf: &[f32]) -> usize {
        buf.chunks(2).position(|f| f[0].abs() > 1e-4).unwrap_or(usize::MAX)
    }

    #[test]
    fn every_sound_is_audible_finite_and_repeatable() {
        for k in ["click", "tick", "pop", "whoosh", "thud", "boom", "riser", "hit", "type", "glitch", "shimmer"] {
            let a = voice(&cue(k, 0.0));
            let peak = a.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(a.iter().all(|s| s.is_finite()), "{k} has NaN/inf");
            assert!(peak > 0.05 && peak < 4.0, "{k} peak {peak}");
            assert_eq!(a, voice(&cue(k, 0.0)), "{k} must be identical on every render");
        }
    }

    #[test]
    fn cues_land_on_their_sample_and_risers_land_on_the_cut() {
        let buf = mix(&[cue("click", 0.5)], 1.0);
        assert_eq!(buf.len(), RATE as usize * 2);
        let at = first_sound(&buf);
        assert!((at as i64 - RATE as i64 / 2).abs() <= 1, "click starts at 0.5 s, got sample {at}");
        // a riser of 1.2 s that lands at 2.0 starts at 0.8 and is silent after 2.0
        let buf = mix(&[cue("riser", 2.0)], 3.0);
        let start = first_sound(&buf) as f64 / RATE as f64;
        assert!(start > 0.75 && start < 1.0, "riser starts {start}");
        assert!(buf[(RATE as usize * 2 + 100) * 2..].iter().all(|s| s.abs() < 1e-6), "nothing after the landing");
    }

    #[test]
    fn loud_scenes_are_kept_under_full_scale_and_wav_is_valid() {
        let many: Vec<Cue> = (0..12).map(|i| cue("hit", 0.1 + i as f64 * 0.001)).collect();
        let buf = mix(&many, 1.0);
        assert!(buf.iter().all(|s| s.abs() <= 0.8901));
        let dir = std::env::temp_dir().join(format!("editemos-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.wav");
        write_wav(&p, &buf).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + buf.len() * 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
