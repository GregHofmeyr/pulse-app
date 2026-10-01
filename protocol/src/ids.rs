//! Typed IDs. ULIDs on the wire as strings, time-sortable.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ulid::Ulid;

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
                Self(Ulid::new())
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
