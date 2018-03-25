//! An implementation of Distance-Based Amplitude Panning as published by Trond Lossius, 2009.

#[derive(Copy, Clone, Debug)]
pub struct Speaker {
    pub distance: f64,
    pub weight: f64,
}

/// An iterator yielding the gain for each given speaker, given their weights and distance from the
/// source position.
#[derive(Clone)]
pub struct SpeakerGains<'a> {
    speakers: &'a [Speaker],
    a_coefficient: f64,
    k_coefficient: f64,
    i: usize,
}

impl<'a> SpeakerGains<'a> {
    /// Given:
    ///
    /// - a list of speaker distances from the virtual source:
    /// - weights for each of those speakers and
    /// - some decibell rolloff
    ///
    /// produce an iterator that returns the gain for each speaker given the source as an input.
    pub fn new(speakers: &'a [Speaker], rolloff_db: f64) -> Self {
        assert!(speakers.len() > 0);
        let a_coefficient = a_coefficient(rolloff_db);
        let k_coefficient = k_coefficient(a_coefficient, speakers);
        SpeakerGains {
            speakers,
            a_coefficient,
            k_coefficient,
            i: 0,
        }
    }
}

impl<'a> Iterator for SpeakerGains<'a> {
    type Item = f64;
    fn next(&mut self) -> Option<Self::Item> {
        let i = self.i;
        if i >= self.speakers.len() {
            return None;
        }
        self.i += 1;
        let s = &self.speakers[i];
        let s_r_amp = v_speaker_relative_amplitude(s, self.k_coefficient, self.a_coefficient);
        Some(s_r_amp / s.distance)
    }
}

/// The relative amplitude for the `i`th speaker where:
///
/// - `k` is a coefficient depending on the position of the source and all speakers
/// - `di` is the distance from the `i`th speaker to the virtual source
/// - `a` is a coefficient calculated from the rolloff `R` in decibels per doubling distance.
fn v_speaker_relative_amplitude(speaker: &Speaker, k: f64, a: f64) -> f64 {
    k * speaker.weight / (2.0 * speaker.distance * a)
}

/// A coefficient calculated form the rolloff `r` in decibels per doubling of distance.
///
/// A rolloff of 6dB equals the inverse distance law for sound propagataing in a free field.
///
/// For closed or semi-closed environments `r` will generally be lower, in the range 3-5dB, and
/// depend on reflections and reverberation.
fn a_coefficient(rolloff_db: f64) -> f64 {
    10f64.powf(-rolloff_db / 20.0)
}

/// `k` is a coefficient depending on the position of the source and all speakers.
fn k_coefficient(a: f64, speakers: &[Speaker]) -> f64 {
    assert!(speakers.len() >= 1);
    2.0 * a
        / speakers
            .iter()
            .fold(0.0, |acc, s| acc + s.weight.powi(2) / s.distance.powi(2))
}

#[test]
fn speaker_gains() {
    use nannou::prelude::*;

    let src = vec2(5.0, 5.0);
    let speaker = |v: Vector2<f64>, w| Speaker {
        distance: v.distance(src),
        weight: w,
    };
    let a = speaker(vec2(0.0, 0.0), 1.0);
    let b = speaker(vec2(10.0, 0.0), 1.0);
    let c = speaker(vec2(10.0, 10.0), 1.0);
    let d = speaker(vec2(0.0, 10.0), 1.0);
    let spkrs = vec![a, b, c, d];
    let r = 6.0; // free-field rolloff db.
    let gains = SpeakerGains::new(&spkrs, r).collect::<Vec<_>>();
    let g = gains[0];
    for gain in gains {
        assert_eq!(g, gain);
    }
}
