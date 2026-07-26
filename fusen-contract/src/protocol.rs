use crate::ContractError;

/// A versioned JSON wire protocol understood by fusen-rs runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum WireProtocol {
    /// Fusen V1 JSON over HTTP/2.
    FusenV1,
    /// Spring Cloud V1 JSON over HTTP/1.1.
    SpringCloudV1,
}

impl WireProtocol {
    /// Returns the stable identifier used by registries, diagnostics, and metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FusenV1 => "fusen-v1",
            Self::SpringCloudV1 => "spring-cloud-v1",
        }
    }
}

impl std::fmt::Display for WireProtocol {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A validated, non-empty set of wire protocols advertised by one provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolSet(u8);

impl ProtocolSet {
    const FUSEN_V1_BIT: u8 = 1 << 0;
    const SPRING_CLOUD_V1_BIT: u8 = 1 << 1;

    /// Only Fusen V1 is enabled.
    pub const FUSEN_V1: Self = Self(Self::FUSEN_V1_BIT);
    /// Only Spring Cloud V1 is enabled.
    pub const SPRING_CLOUD_V1: Self = Self(Self::SPRING_CLOUD_V1_BIT);
    /// Both supported JSON protocols are enabled.
    pub const ALL: Self = Self(Self::FUSEN_V1_BIT | Self::SPRING_CLOUD_V1_BIT);

    /// Builds a non-empty set from protocol values.
    pub fn new(protocols: impl IntoIterator<Item = WireProtocol>) -> Result<Self, ContractError> {
        let mut bits = 0;
        for protocol in protocols {
            bits |= Self::bit(protocol);
        }
        if bits == 0 {
            Err(ContractError::EmptyProtocolSet)
        } else {
            Ok(Self(bits))
        }
    }

    /// Returns a set containing one protocol.
    pub const fn from_protocol(protocol: WireProtocol) -> Self {
        Self(Self::bit(protocol))
    }

    /// Returns whether the set contains `protocol`.
    pub const fn contains(self, protocol: WireProtocol) -> bool {
        self.0 & Self::bit(protocol) != 0
    }

    /// Returns whether every protocol in `self` is also present in `other`.
    pub const fn is_subset_of(self, other: Self) -> bool {
        self.0 & other.0 == self.0
    }

    /// Iterates in stable Fusen V1, Spring Cloud V1 order.
    pub fn iter(self) -> impl Iterator<Item = WireProtocol> {
        [WireProtocol::FusenV1, WireProtocol::SpringCloudV1]
            .into_iter()
            .filter(move |protocol| self.contains(*protocol))
    }

    /// Returns the number of enabled protocols.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Returns `false`; a [`ProtocolSet`] is always non-empty.
    pub const fn is_empty(self) -> bool {
        false
    }

    const fn bit(protocol: WireProtocol) -> u8 {
        match protocol {
            WireProtocol::FusenV1 => Self::FUSEN_V1_BIT,
            WireProtocol::SpringCloudV1 => Self::SPRING_CLOUD_V1_BIT,
        }
    }
}

impl Default for ProtocolSet {
    fn default() -> Self {
        Self::FUSEN_V1
    }
}

impl From<WireProtocol> for ProtocolSet {
    fn from(protocol: WireProtocol) -> Self {
        Self::from_protocol(protocol)
    }
}

/// Retry and HTTP-safety semantics declared for one RPC method.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Idempotency {
    /// Repeating the call can produce additional side effects.
    #[default]
    None,
    /// Repeating the call has the same intended effect as one call.
    Idempotent,
    /// The call is read-only and is also idempotent.
    Safe,
}

impl Idempotency {
    /// Returns whether an identical call may be repeated without additional intended effects.
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Self::Idempotent | Self::Safe)
    }

    /// Returns whether the call is declared read-only.
    pub const fn is_safe(self) -> bool {
        matches!(self, Self::Safe)
    }

    /// Returns the stable wire and diagnostic value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Idempotent => "idempotent",
            Self::Safe => "safe",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_set_is_non_empty_and_iterates_stably() {
        assert_eq!(ProtocolSet::new([]), Err(ContractError::EmptyProtocolSet));
        let protocols = ProtocolSet::new([
            WireProtocol::SpringCloudV1,
            WireProtocol::FusenV1,
            WireProtocol::SpringCloudV1,
        ])
        .unwrap();
        assert_eq!(protocols, ProtocolSet::ALL);
        assert_eq!(
            protocols.iter().collect::<Vec<_>>(),
            [WireProtocol::FusenV1, WireProtocol::SpringCloudV1]
        );
        assert_eq!(protocols.len(), 2);
        assert!(!protocols.is_empty());
        assert!(ProtocolSet::FUSEN_V1.is_subset_of(ProtocolSet::ALL));
        assert!(!ProtocolSet::ALL.is_subset_of(ProtocolSet::FUSEN_V1));
    }

    #[test]
    fn protocol_and_idempotency_names_are_stable() {
        assert_eq!(WireProtocol::FusenV1.as_str(), "fusen-v1");
        assert_eq!(WireProtocol::SpringCloudV1.as_str(), "spring-cloud-v1");
        assert!(!Idempotency::None.is_idempotent());
        assert!(Idempotency::Idempotent.is_idempotent());
        assert!(Idempotency::Safe.is_idempotent());
        assert!(Idempotency::Safe.is_safe());
    }
}
