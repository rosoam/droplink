use std::net::Ipv4Addr;

/// Minimal interface description used by the pure selection logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub is_up: bool,
}

const EXCLUDED_PREFIXES: &[&str] = &[
    "utun", "tun", "tap", "docker", "vmnet", "vboxnet", "awdl", "llw", "veth", "br-",
    // Cellular. Advertising the 3G/4G address in Bonjour makes peers dial an address that is
    // not reachable from the local link (and can make the publish itself fail).
    "pdp_ip", "cellular", "rmnet",
];

/// Interface-name preference, added to the address score.
///
/// On iOS the Personal Hotspot AP is `bridge100` (172.20.10.1) while `en0` is the Wi-Fi client
/// interface. Prefer a physical Wi-Fi address, then the hotspot bridge; cellular is excluded.
fn name_score(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    if lower == "en0" {
        3
    } else if lower.starts_with("bridge") {
        2
    } else {
        1
    }
}

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
        .max_by_key(|i| (score(&i.ip), name_score(&i.name)))
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

/// Vrai pour l'adresse d'un client du partage de connexion iPhone (`172.20.10.0/28`).
///
/// Dans ce réseau, le multicast mDNS est filtré de façon intermittente entre l'iPhone
/// et ses clients : la découverte automatique peut échouer alors que les deux
/// appareils restent joignables en TCP. L'interface le signale pour proposer la
/// connexion par adresse.
pub fn is_personal_hotspot(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 172 && octets[1] == 20 && octets[2] == 10 && octets[3] < 16
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
    fn cellular_only_returns_none() {
        let ifaces = vec![iface("pdp_ip0", [10, 209, 199, 215])];
        assert_eq!(select_advertisable_ip(&ifaces), None);
    }

    #[test]
    fn hotspot_bridge_is_usable_and_beats_no_interface() {
        let ifaces = vec![iface("bridge100", [172, 20, 10, 1])];
        assert_eq!(
            select_advertisable_ip(&ifaces),
            Some(Ipv4Addr::new(172, 20, 10, 1))
        );
    }

    #[test]
    fn en0_beats_a_bridge_on_equal_addresses() {
        let ifaces = vec![
            iface("bridge100", [192, 168, 1, 2]),
            iface("en0", [192, 168, 1, 2]),
        ];
        assert_eq!(
            select_advertisable_ip(&ifaces),
            Some(Ipv4Addr::new(192, 168, 1, 2))
        );
    }

    #[test]
    fn hotspot_wins_over_cellular() {
        let ifaces = vec![
            iface("pdp_ip0", [10, 209, 199, 215]),
            iface("en0", [172, 20, 10, 1]),
        ];
        assert_eq!(
            select_advertisable_ip(&ifaces),
            Some(Ipv4Addr::new(172, 20, 10, 1))
        );
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
    fn detects_iphone_personal_hotspot() {
        assert!(is_personal_hotspot(&Ipv4Addr::new(172, 20, 10, 1)));
        assert!(is_personal_hotspot(&Ipv4Addr::new(172, 20, 10, 7)));
        assert!(!is_personal_hotspot(&Ipv4Addr::new(172, 20, 11, 7)));
        assert!(!is_personal_hotspot(&Ipv4Addr::new(192, 168, 1, 7)));
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
