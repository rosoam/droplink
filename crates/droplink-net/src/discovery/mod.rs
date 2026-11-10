pub mod interfaces;
pub mod mdns;

pub use interfaces::{is_personal_hotspot, select_advertisable_ip, system_interfaces, Iface};
pub use mdns::{advert_for, AdvertSpec, DiscoveryEvent, DiscoveryHandle};
