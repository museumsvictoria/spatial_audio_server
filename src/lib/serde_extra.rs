pub mod atomic {
    use atomic::{self, Atomic};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn deserialize<'de, D, T>(d: D) -> Result<Atomic<T>, D::Error>
        where D: Deserializer<'de>,
              T: Copy + Deserialize<'de>,
    {
        let t = T::deserialize(d)?;
        Ok(Atomic::new(t))
    }
    
    pub fn serialize<T, S>(role: &Atomic<T>, s: S) -> Result<S::Ok, S::Error>
        where T: Copy + Serialize,
              S: Serializer,
    {
        let t = role.load(atomic::Ordering::Relaxed);
        t.serialize(s)
    }
}

pub mod atomic_usize {
    use atomic;
    use std::sync::atomic::AtomicUsize;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn deserialize<'de, D>(d: D) -> Result<AtomicUsize, D::Error>
        where D: Deserializer<'de>,
    {
        let c = usize::deserialize(d)?;
        Ok(AtomicUsize::new(c))
    }
    
    pub fn serialize<S>(channel: &AtomicUsize, s: S) -> Result<S::Ok, S::Error>
        where S: Serializer,
    {
        let c = channel.load(atomic::Ordering::Relaxed);
        c.serialize(s)
    }
}
