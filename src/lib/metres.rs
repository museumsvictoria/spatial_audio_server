custom_derive! {
    /// Used for describing locations throughout the installation.
    ///
    /// Using this GUI agnostic measurement greatly simplifies positioning and makes it easier
    #[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd, NewtypeFrom,
             NewtypeAdd, NewtypeSub, NewtypeMul, NewtypeMul(f64), NewtypeDiv, NewtypeDiv(f64),
             NewtypeAddAssign, NewtypeSubAssign, NewtypeMulAssign, NewtypeDivAssign,
             NewtypeMulAssign(f64), NewtypeDivAssign(f64),
             NewtypeRem, NewtypeRemAssign,
             NewtypeNeg)]
    pub struct Metres(pub f64);
}

impl Metres {
    pub fn min(self, other: Self) -> Self {
        if other < self { other } else { self }
    }

    pub fn max(self, other: Self) -> Self {
        if other > self { other } else { self }
    }
}
