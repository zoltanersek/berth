use crate::types::Config;
use std::{collections::HashMap, net::TcpListener};

/// Allocate a free TCP port for every port declared in the config.
///
/// Each port is chosen by binding to `127.0.0.1:0` and asking the OS for an
/// ephemeral port. We keep every listener bound until *all* ports have been
/// chosen, so the OS cannot hand out the same port twice within a single call.
/// The listeners are dropped when this function returns, immediately before the
/// caller hands the ports to Docker. A small time-of-check/time-of-use window
/// therefore remains between allocation and `docker up` binding the port — this
/// is inherent to the bind-port-0 technique and acceptable for local dev use.
pub fn allocate(config: &Config) -> Result<HashMap<String, u16>, String> {
    let mut ports = HashMap::new();
    // Hold the listeners so their ports stay reserved for the whole loop.
    let mut listeners = Vec::new();

    for port in config.ports.values() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to allocate port: {e}"))?;

        let assigned = listener
            .local_addr()
            .map_err(|e| format!("Failed to determine allocated port: {e}"))?
            .port();

        ports.insert(port.env.clone(), assigned);
        listeners.push(listener);
    }

    Ok(ports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PortConfig;
    use std::collections::HashSet;

    fn config_with(envs: &[&str]) -> Config {
        let ports = envs
            .iter()
            .map(|e| (e.to_string(), PortConfig { env: e.to_string() }))
            .collect();

        Config {
            version: 1,
            compose: "docker-compose.yml".to_string(),
            ports,
        }
    }

    #[test]
    fn allocates_one_port_per_entry() {
        let config = config_with(&["A", "B", "C"]);
        let ports = allocate(&config).unwrap();
        assert_eq!(ports.len(), 3);
        assert!(ports.contains_key("A"));
        assert!(ports.contains_key("B"));
        assert!(ports.contains_key("C"));
    }

    #[test]
    fn allocated_ports_are_distinct_and_nonzero() {
        // The core guarantee: holding every listener until all ports are chosen
        // means the OS cannot hand out the same ephemeral port twice.
        let config = config_with(&["A", "B", "C", "D", "E", "F", "G", "H"]);
        let ports = allocate(&config).unwrap();

        let unique: HashSet<u16> = ports.values().copied().collect();
        assert_eq!(unique.len(), ports.len(), "ports must be unique");
        assert!(ports.values().all(|&p| p != 0), "ports must be non-zero");
    }

    #[test]
    fn empty_ports_yields_empty_map() {
        let config = config_with(&[]);
        assert!(allocate(&config).unwrap().is_empty());
    }
}
