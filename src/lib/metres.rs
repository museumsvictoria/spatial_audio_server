pub type Metres = f64; //(pub f64);

pub trait Metric {
    fn min(self: Self, other: Self) -> Self;
    fn max(self: Self, other: Self) -> Self;
}

impl Metric for Metres {
    fn min(self, other: Self) -> Self {
        if other < self {
            other
        } else {
            self
        }
    }

    fn max(self, other: Self) -> Self {
        if other > self {
            other
        } else {
            self
        }
    }
}
