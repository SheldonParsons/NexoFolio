use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Non-serializable, redacted credentials. Access explicitly at adapter boundaries.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_does_not_reveal_credentials() {
        assert_eq!(
            format!("{:?}", Secret::new("sensitive")),
            "Secret([REDACTED])"
        );
    }
}
