//! Internal helper: zero-sized "literal string tag" types.
//!
//! Several RC shapes are discriminated by a required field that must hold an
//! exact string (e.g. `"jsonrpc": "2.0"`, `"resultType": "inputRequired"`).
//! Modelling those fields as `String` would allow constructing — and silently
//! accepting — invalid envelopes. A unit struct that serializes to exactly one
//! literal and refuses anything else on deserialize makes the invalid states
//! unrepresentable at zero runtime cost.

/// Define a zero-sized struct that round-trips as one exact JSON string.
macro_rules! string_tag {
    ($(#[$attr:meta])* $vis:vis struct $name:ident = $lit:literal;) => {
        $(#[$attr])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        $vis struct $name;

        impl $name {
            /// The exact wire value this tag serializes to.
            pub const VALUE: &'static str = $lit;
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str($lit)
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                if s == $lit {
                    Ok(Self)
                } else {
                    Err(<D::Error as ::serde::de::Error>::custom(format!(
                        concat!("expected the literal string \"", $lit, "\", got {:?}"),
                        s
                    )))
                }
            }
        }
    };
}

pub(crate) use string_tag;
