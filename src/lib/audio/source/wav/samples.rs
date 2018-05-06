use hound;
use nannou::audio::sample::{FromSample, Sample};
use std::io;

/// Retrieve the next sample from the given wav reader samples iterator and yield it in the sample
/// format type `S`.
pub fn next<'a, R, H, S>(samples: &mut hound::WavSamples<'a, R, H>) -> Option<Result<S, hound::Error>>
where
    H: hound::Sample + Sample,
    S: Sample + FromSample<H>,
    hound::WavSamples<'a, R, H>: Iterator<Item = Result<H, hound::Error>>,
{
    samples
        .next()
        .map(|r| r.map(Sample::to_sample))
}

/// The number of remaining samples in the reader from its current position.
pub fn remaining<R>(reader: &mut hound::WavReader<R>) -> usize
where
    R: io::Read,
{
    let spec = reader.spec();
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader.samples::<f32>().len(),
        (hound::SampleFormat::Int, 8) => reader.samples::<i8>().len(),
        (hound::SampleFormat::Int, 16) => reader.samples::<i16>().len(),
        (hound::SampleFormat::Int, 32) => reader.samples::<i32>().len(),
        _ => panic!("unsupported WAV sample format or bits per sample"),
    }
}

// /// A wrapper around the possible hound samples iterators that may produce an iterator yielding
// /// samples of any `Sample` type.
// pub enum WavSamples<'a, R>
// where
//     R: 'a,
// {
//     I8(hound::WavSamples<'a, R, i8>),
//     I16(hound::WavSamples<'a, R, i16>),
//     I32(hound::WavSamples<'a, R, i32>),
//     F32(hound::WavSamples<'a, R, f32>),
// }
// 
// /// The `WavSamples` iterator.
// pub struct WavSamplesIter<'a, R, S>
// where
//     R: 'a,
// {
//     wav_samples: WavSamples<'a, R>,
//     sample_type: PhantomData<S>,
// }
// 
// impl<'a, R> WavSamples<'a, R>
// where
//     R: io::Read,
// {
//     /// Yield the next sample from the reader and convert it to the sample type `S`.
//     pub fn next<S>(&mut self) -> Option<S>
//     where
//         S: Sample,
//     {
//         match *self {
//             WavSamples::I8(ref mut r) => r.next().map(|r| r.map(Sample::from_sample)),
//             WavSamples::I16(ref mut r) => r.next().map(|r| r.map(Sample::from_sample)),
//             WavSamples::I32(ref mut r) => r.next().map(|r| r.map(Sample::from_sample)),
//             WavSamples::F32(ref mut r) => r.next().map(|r| r.map(Sample::from_sample)),
//         }
//     }
// 
//     /// The exact number of remaining items in the iterator.
//     pub fn len(&self) -> usize
//     where
//         hound::WavSamples<'a, R, i8>: ExactSizeIterator,
//         hound::WavSamples<'a, R, i16>: ExactSizeIterator,
//         hound::WavSamples<'a, R, i32>: ExactSizeIterator,
//         hound::WavSamples<'a, R, f32>: ExactSizeIterator,
//     {
//         match *self {
//             WavSamples::I8(ref r) => r.len(),
//             WavSamples::I16(ref r) => r.len(),
//             WavSamples::I32(ref r) => r.len(),
//             WavSamples::F32(ref r) => r.len(),
//         }
//     }
// }
// 
// macro_rules! impl_from {
//     ($S:ty, $Variant:ident) => {
//         impl<'a, R> From<hound::WavSamples<'a, R, $S>> for WavSamples<'a, R> {
//             fn from(r: hound::WavSamples<'a, R, $S>) -> Self {
//                 WavSamples::$Variant(r)
//             }
//         }
//     };
// }
// 
// impl_from!(i8, I8);
// impl_from!(i16, I16);
// impl_from!(i32, I32);
// impl_from!(f32, F32);
// 
// impl<'a, R, S> Iterator for WavSamplesIter<'a, R, S>
// where
//     R: io::Read,
//     S: Sample
// {
//     type Item = S;
//     fn next(&mut self) -> Option<Self::Item> {
//         self.wav_samples.next()
//     }
// 
//     fn size_hint(&self) -> (usize, Option<usize>) {
//         match self.wav_samples {
//             WavSamples::I8(ref r) => r.size_hint(),
//             WavSamples::I16(ref r) => r.size_hint(),
//             WavSamples::I32(ref r) => r.size_hint(),
//             WavSamples::F32(ref r) => r.size_hint(),
//         }
//     }
// }
// 
// impl<'a, R, S> ExactSizeIterator for WavSamplesIter<'a, R, S>
// where
//     R: io::Read,
//     S: Sample,
//     hound::WavSamples<'a, R, i8>: ExactSizeIterator,
//     hound::WavSamples<'a, R, i16>: ExactSizeIterator,
//     hound::WavSamples<'a, R, i32>: ExactSizeIterator,
//     hound::WavSamples<'a, R, f32>: ExactSizeIterator,
// {
//     fn len(&self) -> usize {
//         self.wav_sample.len()
//     }
// }
