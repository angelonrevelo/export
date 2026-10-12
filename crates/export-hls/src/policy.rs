use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::error::Error;

/// Loopback and link-local policy applied before connect and on every redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConnectPolicy {
    /// When `false`, loopback addresses are refused. Link-local addresses are
    /// always refused.
    pub allow_loopback: bool,
}

impl ConnectPolicy {
    pub fn new(allow_loopback: bool) -> Self {
        Self { allow_loopback }
    }

    /// Refuse loopback unless `allow_loopback` is set, and always refuse
    /// link-local. IPv4-mapped IPv6 addresses are classified as IPv4.
    pub fn check_ip(self, ip: IpAddr) -> Result<(), Error> {
        if let Some(v4) = ipv4_view(ip) {
            if is_ipv4_loopback(v4) {
                return if self.allow_loopback {
                    Ok(())
                } else {
                    Err(Error::Blocked)
                };
            }
            if is_ipv4_link_local(v4) {
                return Err(Error::Blocked);
            }
            return Ok(());
        }
        if let IpAddr::V6(v6) = ip {
            if is_ipv6_loopback(v6) {
                return if self.allow_loopback {
                    Ok(())
                } else {
                    Err(Error::Blocked)
                };
            }
            if is_ipv6_link_local(v6) {
                return Err(Error::Blocked);
            }
        }
        Ok(())
    }
}

fn ipv4_view(ip: IpAddr) -> Option<Ipv4Addr> {
    match ip {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(v6) => mapped_v4(v6),
    }
}

fn mapped_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    let octets = v6.octets();
    let prefix_clear = octets[..10].iter().all(|byte| *byte == 0);
    if prefix_clear && octets[10] == 0xff && octets[11] == 0xff {
        Some(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ))
    } else {
        None
    }
}

fn is_ipv4_loopback(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 127
}

fn is_ipv4_link_local(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 169 && octets[1] == 254
}

fn is_ipv6_loopback(ip: Ipv6Addr) -> bool {
    ip == Ipv6Addr::LOCALHOST
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_loopback_unless_allowed() {
        let deny = ConnectPolicy::default();
        assert_eq!(
            deny.check_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
            Err(Error::Blocked)
        );
        let allow = ConnectPolicy::new(true);
        assert_eq!(
            allow.check_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 8))),
            Ok(())
        );
    }

    #[test]
    fn link_local_stays_blocked_when_loopback_is_allowed() {
        let allow = ConnectPolicy::new(true);
        assert_eq!(
            allow.check_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))),
            Err(Error::Blocked)
        );
        assert_eq!(
            allow.check_ip(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))),
            Err(Error::Blocked)
        );
    }

    #[test]
    fn mapped_loopback_is_loopback() {
        let deny = ConnectPolicy::default();
        let mapped = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 1);
        assert_eq!(deny.check_ip(IpAddr::V6(mapped)), Err(Error::Blocked));
    }
}
