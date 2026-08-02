use crate::resilience::breaker::{
    BreakerConfig, BreakerPhase, CircuitBreaker, EndpointBreakerStore,
};
use fusen_contract::{HttpBindingId, ServiceInstance, ServiceSelector};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

type TransitionObserver = Arc<dyn Fn(BreakerPhase) + Send + Sync + 'static>;

/// Separates explicitly configured endpoints from registry-owned discovery membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EndpointBreakerSource {
    Direct,
    Discovery,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EndpointBreakerKey {
    service: String,
    binding_id: HttpBindingId,
    source: EndpointBreakerSource,
    endpoint: String,
}

impl EndpointBreakerKey {
    fn new(
        service: &str,
        binding_id: &HttpBindingId,
        source: EndpointBreakerSource,
        endpoint: &str,
    ) -> Self {
        Self {
            service: service.to_owned(),
            binding_id: binding_id.clone(),
            source,
            endpoint: endpoint.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DiscoveryOwner {
    selector: ServiceSelector,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DiscoveryEndpoint {
    service: String,
    endpoint: String,
}

#[derive(Debug)]
struct EndpointBreakersInner {
    store: EndpointBreakerStore<EndpointBreakerKey>,
    memberships: Mutex<DiscoveryMemberships>,
}

#[derive(Debug, Default)]
struct DiscoveryMemberships {
    owners: HashMap<DiscoveryOwner, HashSet<DiscoveryEndpoint>>,
    references: HashMap<DiscoveryEndpoint, usize>,
}

/// Runtime-owned endpoint breaker cache and discovery membership index.
///
/// The membership lock is always acquired before the store lock. Discovery lookup holds both
/// across membership validation and insertion, so a concurrent snapshot update cannot resurrect
/// an endpoint after removing it from the active membership.
#[derive(Clone, Debug)]
pub(crate) struct EndpointBreakers {
    inner: Arc<EndpointBreakersInner>,
}

impl EndpointBreakers {
    pub(crate) fn new(config: BreakerConfig, max_entries: usize, idle_eviction: Duration) -> Self {
        Self {
            inner: Arc::new(EndpointBreakersInner {
                store: EndpointBreakerStore::new(config, max_entries, idle_eviction),
                memberships: Mutex::new(DiscoveryMemberships::default()),
            }),
        }
    }

    pub(crate) fn get_or_insert_observed(
        &self,
        service: &str,
        binding_id: &HttpBindingId,
        source: EndpointBreakerSource,
        endpoint: &str,
        observer: TransitionObserver,
    ) -> Arc<CircuitBreaker> {
        let key = EndpointBreakerKey::new(service, binding_id, source, endpoint);
        if source == EndpointBreakerSource::Direct {
            return self.inner.store.get_or_insert_observed(key, observer);
        }

        let memberships = self
            .inner
            .memberships
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let member = DiscoveryEndpoint {
            service: service.to_owned(),
            endpoint: endpoint.to_owned(),
        };
        if memberships.references.contains_key(&member) {
            self.inner.store.get_or_insert_observed(key, observer)
        } else {
            self.inner.store.untracked_observed(observer)
        }
    }

    pub(crate) fn replace_discovery(
        &self,
        selector: &ServiceSelector,
        instances: &[ServiceInstance],
    ) {
        let owner = DiscoveryOwner {
            selector: selector.clone(),
        };
        let current = instances
            .iter()
            .map(|instance| DiscoveryEndpoint {
                service: selector.identity().to_owned(),
                endpoint: instance.endpoint().as_str().to_owned(),
            })
            .collect::<HashSet<_>>();
        let mut memberships = self
            .inner
            .memberships
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = memberships.owners.remove(&owner).unwrap_or_default();
        let evicted =
            decrement_references(&mut memberships.references, previous.difference(&current));
        increment_references(&mut memberships.references, current.difference(&previous));
        memberships.owners.insert(owner, current);
        self.remove_cached(evicted);
    }

    pub(crate) fn remove_discovery(&self, selector: &ServiceSelector) {
        let owner = DiscoveryOwner {
            selector: selector.clone(),
        };
        let mut memberships = self
            .inner
            .memberships
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let removed = memberships.owners.remove(&owner).unwrap_or_default();
        let evicted = decrement_references(&mut memberships.references, removed.iter());
        self.remove_cached(evicted);
    }

    fn remove_cached(&self, endpoints: Vec<DiscoveryEndpoint>) {
        let endpoints = endpoints.into_iter().collect::<HashSet<_>>();
        self.inner.store.retain(|key| {
            key.source != EndpointBreakerSource::Discovery
                || !endpoints.contains(&DiscoveryEndpoint {
                    service: key.service.clone(),
                    endpoint: key.endpoint.clone(),
                })
        });
    }
}

fn increment_references<'a>(
    references: &mut HashMap<DiscoveryEndpoint, usize>,
    endpoints: impl Iterator<Item = &'a DiscoveryEndpoint>,
) {
    for endpoint in endpoints {
        let count = references.entry(endpoint.clone()).or_default();
        *count = count.saturating_add(1);
    }
}

fn decrement_references<'a>(
    references: &mut HashMap<DiscoveryEndpoint, usize>,
    endpoints: impl Iterator<Item = &'a DiscoveryEndpoint>,
) -> Vec<DiscoveryEndpoint> {
    let mut evicted = Vec::new();
    for endpoint in endpoints {
        let remove = match references.get_mut(endpoint) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if remove {
            references.remove(endpoint);
            evicted.push(endpoint.clone());
        }
    }
    evicted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resilience::breaker::DEFAULT_ENDPOINT_IDLE_EVICTION;
    use fusen_contract::{
        EndpointCapabilities, HttpBindingId, InstanceId, ServiceEndpoint, ServiceWeight,
    };

    fn breakers() -> EndpointBreakers {
        EndpointBreakers::new(
            BreakerConfig::new(
                Duration::from_secs(10),
                10,
                20,
                0.5,
                Duration::from_secs(10),
                Duration::from_secs(120),
                1,
                2,
            ),
            10_000,
            DEFAULT_ENDPOINT_IDLE_EVICTION,
        )
    }

    fn selector() -> ServiceSelector {
        ServiceSelector::new("users", Some("prod".to_owned()), Some("1".to_owned())).unwrap()
    }

    fn instance(id: &str, port: u16) -> ServiceInstance {
        ServiceInstance::new(
            InstanceId::new(id).unwrap(),
            format!("http://127.0.0.1:{port}")
                .parse::<ServiceEndpoint>()
                .unwrap(),
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        )
    }

    fn observer() -> TransitionObserver {
        Arc::new(|_| {})
    }

    #[test]
    fn snapshot_removal_evicts_only_the_missing_discovery_entry() {
        let breakers = breakers();
        let selector = selector();
        let binding = HttpBindingId::default();
        let first = instance("first", 8001);
        let second = instance("second", 8002);
        breakers.replace_discovery(&selector, &[first.clone(), second.clone()]);
        let removed = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            first.endpoint().as_str(),
            observer(),
        );
        let retained = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            second.endpoint().as_str(),
            observer(),
        );

        breakers.replace_discovery(&selector, std::slice::from_ref(&second));

        let removed_replacement = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            first.endpoint().as_str(),
            observer(),
        );
        let retained_again = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            second.endpoint().as_str(),
            observer(),
        );
        assert!(!Arc::ptr_eq(&removed, &removed_replacement));
        assert!(Arc::ptr_eq(&retained, &retained_again));
    }

    #[test]
    fn discovery_cleanup_does_not_prune_direct_or_other_owner_entries() {
        let breakers = breakers();
        let selector = selector();
        let binding = HttpBindingId::default();
        let endpoint = instance("shared", 8001);
        let mut metadata = fusen_contract::Metadata::new();
        metadata.insert("zone".to_owned(), "east".to_owned());
        let filtered_selector = selector.clone().with_metadata(metadata).unwrap();
        breakers.replace_discovery(&selector, std::slice::from_ref(&endpoint));
        breakers.replace_discovery(&filtered_selector, std::slice::from_ref(&endpoint));
        let discovered = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            endpoint.endpoint().as_str(),
            observer(),
        );
        let direct = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Direct,
            endpoint.endpoint().as_str(),
            observer(),
        );

        breakers.remove_discovery(&selector);
        let still_discovered = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            endpoint.endpoint().as_str(),
            observer(),
        );
        assert!(Arc::ptr_eq(&discovered, &still_discovered));

        breakers.remove_discovery(&filtered_selector);
        let uncached_discovery = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Discovery,
            endpoint.endpoint().as_str(),
            observer(),
        );
        let direct_again = breakers.get_or_insert_observed(
            selector.identity(),
            &binding,
            EndpointBreakerSource::Direct,
            endpoint.endpoint().as_str(),
            observer(),
        );
        assert!(!Arc::ptr_eq(&discovered, &uncached_discovery));
        assert!(Arc::ptr_eq(&direct, &direct_again));
    }
}
