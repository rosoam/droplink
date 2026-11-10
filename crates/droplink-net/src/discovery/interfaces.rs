use std::net::Ipv4Addr;

/// Minimal interface description used by the pure selection logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub is_up: bool,
}

const EXCLUDED_PREFIXES: &[&str] = &[
    "utun", "tun", "tap", "docker", "vmnet", "vboxnet", "bridge", "awdl", "llw", "veth", "br-",
];

/// Pick the address to advertise over mDNS.
///
/// Never returns loopback, link-local or unspecified addresses. A `None` result disables mDNS
/// instead of falling back to `127.0.0.1` (the bug in the previous implementation).
pub fn select_advertisable_ip(ifaces: &[Iface]) -> Option<Ipv4Addr> {
    ifaces
        .iter()
        .filter(|i| i.is_up)
        .filter(|i| !is_excluded_name(&i.name))
        .filter(|i| is_usable(&i.ip))
        .max_by_key(|i| score(&i.ip))
        .map(|i| i.ip)
}

fn is_excluded_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    EXCLUDED_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn is_usable(ip: &Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() && !ip.is_broadcast()
}

fn score(ip: &Ipv4Addr) -> u8 {
    if is_private(ip) {
        3
    } else if ip.is_documentation() || ip.is_multicast() {
        0
    } else {
        1
    }
}

fn is_private(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 10
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 168)
}

/// Enumerate the host interfaces through `if-addrs`.
pub fn system_interfaces() -> Vec<Iface> {
    match if_addrs::get_if_addrs() {
        Ok(ifaces) => ifaces
            .into_iter()
            .filter_map(|iface| match iface.addr {
                if_addrs::IfAddr::V4(v4) => Some(Iface {
                    name: iface.name,
                    ip: v4.ip,
                    is_up: true,
                }),
                if_addrs::IfAddr::V6(_) => None,
            })
            .collect(),
        Err(err) => {
            tracing::warn!(error = %err, "interface enumeration failed");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, ip: [u8; 4]) -> Iface {
        Iface {
            name: name.into(),
            ip: Ipv4Addr::from(ip),
            is_up: true,
        }
    }

    #[test]
    fn isolated_lan_returns_none() {
        let ifaces = vec![
            iface("lo0", [127, 0, 0, 1]),
            iface("llw0", [169, 254, 1, 2]),
        ];
        assert_eq!(select_advertisable_ip(&ifaces), None);
    }

    #[test]
    fn vpn_only_returns_none() {
        let ifaces = vec![iface("lo0", [127, 0, 0, 1]), iface("utun3", [10, 8, 0, 2])];
        assert_eq!(select_advertisable_ip(&ifaces), None);
    }

    #[test]
    fn docker_and_en0_picks_en0() {
        let ifaces = vec![
            iface("docker0", [172, 17, 0, 1]),
            iface("en0", [192, 168, 1, 20]),
        ];
        assert_eq!(
            select_advertisable_ip(&ifaces),
            Some(Ipv4Addr::new(192, 168, 1, 20))
        );
    }

    #[test]
    fn prefers_private_over_public() {
        let ifaces = vec![iface("en0", [203, 0, 113, 5]), iface("en1", [10, 0, 0, 7])];
        assert_eq!(
            select_advertisable_ip(&ifaces),
            Some(Ipv4Addr::new(10, 0, 0, 7))
        );
    }

    #[test]
    fn ignores_down_interfaces() {
        let mut down = iface("en0", [192, 168, 1, 20]);
        down.is_up = false;
        assert_eq!(select_advertisable_ip(&[down]), None);
    }

    #[test]
    fn ignores_link_local_and_broadcast() {
        let ifaces = vec![
            iface("en0", [169, 254, 10, 1]),
            iface("en1", [255, 255, 255, 255]),
        ];
        assert_eq!(select_advertisable_ip(&ifaces), None);
    }
}
