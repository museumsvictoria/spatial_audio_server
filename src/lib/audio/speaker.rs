use atomic::{self, Atomic};
use cgmath::Point2;
use metres::Metres;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::atomic::AtomicUsize;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub u64);

/// Represents a virtual output at some location within the space.
///
/// These parameters are atomics in order to safely share them with the GUI thread.
#[derive(Deserialize, Serialize)]
pub struct Speaker {
    // The location of the speaker within the space.
    #[serde(deserialize_with = "deserialize_point", serialize_with = "serialize_point")]
    pub point: Atomic<Point2<Metres>>,
    // The channel on which the output is rendered.
    #[serde(deserialize_with = "deserialize_channel", serialize_with = "serialize_channel")]
    pub channel: AtomicUsize,
}

fn deserialize_point<'de, D>(d: D) -> Result<Atomic<Point2<Metres>>, D::Error>
    where D: Deserializer<'de>,
{
    let p = Point2::<Metres>::deserialize(d)?;
    Ok(Atomic::new(p))
}

fn deserialize_channel<'de, D>(d: D) -> Result<AtomicUsize, D::Error>
    where D: Deserializer<'de>,
{
    let c = usize::deserialize(d)?;
    Ok(AtomicUsize::new(c))
}

fn serialize_point<S>(point: &Atomic<Point2<Metres>>, s: S) -> Result<S::Ok, S::Error>
    where S: Serializer,
{
    let p = point.load(atomic::Ordering::Relaxed);
    p.serialize(s)
}

fn serialize_channel<S>(channel: &AtomicUsize, s: S) -> Result<S::Ok, S::Error>
    where S: Serializer,
{
    let c = channel.load(atomic::Ordering::Relaxed);
    c.serialize(s)
}
