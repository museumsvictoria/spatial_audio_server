//! Items related to Soundscape Groups.
//!
//! Soundscape groups allow for describing rules/constraints for multiple sounds at once.

/// A name for a soundscape group.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Name(pub String);

/// A more efficient unique identifier for a group.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Id(pub usize);
