//! Typed IDs. ULIDs on the wire as strings, time-sortable.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ulid::Ulid;

/// Monotonic ULIDs: within one millisecond the random part is incremented instead of re-rolled,
/// so IDs minted by this process always sort in creation order (they are the paging cursor).
pub fn next_ulid() -> Ulid {
    static GEN: std::sync::Mutex<Option<ulid::Generator>> = std::sync::Mutex::new(None);
    let mut g = GEN.lock().unwrap_or_else(|p| p.into_inner());
    // Overflow needs 2^80 IDs in one millisecond; fall back to a fresh ULID if it ever happens.
    g.get_or_insert_with(ulid::Generator::new)
        .generate()
        .unwrap_or_else(|_| Ulid::new())
}

macro_rules! id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS,
        )]
        #[ts(type = "string")]
        pub struct $name(pub Ulid);

        impl $name {
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(next_ulid())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(s.parse()?))
            }
        }
    };
}

id!(UserId);
id!(ServerId);
id!(ChannelId);
id!(MessageId);
id!(RoleId);
