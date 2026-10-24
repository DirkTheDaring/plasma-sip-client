//! RFC 3263 DNS SRV Resolution for SIP Server Location (_sip._udp).

use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::{debug, trace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// Builds a standard DNS query packet for SRV record (QTYPE 33, QCLASS IN 1).
pub fn build_srv_query(name: &str, tx_id: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(64);
    // Header (12 bytes)
    packet.extend_from_slice(&tx_id.to_be_bytes()); // ID
    packet.extend_from_slice(&0x0100u16.to_be_bytes()); // Flags: RD=1 (Recursion Desired)
    packet.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT = 1
    packet.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT = 0
    packet.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT = 0
    packet.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT = 0

    // Question Section
    for label in name.split('.') {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            packet.push(trimmed.len() as u8);
            packet.extend_from_slice(trimmed.as_bytes());
        }
    }
    packet.push(0); // Terminating null label
    packet.extend_from_slice(&33u16.to_be_bytes()); // QTYPE = 33 (SRV)
    packet.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = 1 (IN)
    packet
}

/// Parses a domain name from a DNS packet, handling pointer compression (0xC0...).
pub fn parse_dns_name(buf: &[u8], mut offset: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut jumped = false;
    let mut next_offset = offset;
    let mut jumps = 0;

    while offset < buf.len() {
        let len = buf[offset] as usize;
        if len == 0 {
            if !jumped {
                next_offset = offset + 1;
            }
            break;
        }

        if (len & 0xC0) == 0xC0 {
            if offset + 1 >= buf.len() {
                return None;
            }
            let ptr = ((len & 0x3F) << 8) | (buf[offset + 1] as usize);
            if !jumped {
                next_offset = offset + 2;
                jumped = true;
            }
            offset = ptr;
            jumps += 1;
            if jumps > 10 {
                return None; // Prevent pointer loop
            }
            continue;
        }

        offset += 1;
        if offset + len > buf.len() {
            return None;
        }
        let label = std::str::from_utf8(&buf[offset..offset + len]).ok()?;
        labels.push(label);
        offset += len;
        if !jumped {
            next_offset = offset;
        }
    }

    Some((labels.join("."), next_offset))
}

/// Parses a DNS SRV response packet.
pub fn parse_srv_response(buf: &[u8], expected_tx_id: u16) -> Option<Vec<SrvRecord>> {
    if buf.len() < 12 {
        return None;
    }

    let tx_id = u16::from_be_bytes([buf[0], buf[1]]);
    if tx_id != expected_tx_id {
        return None;
    }

    let flags = u16::from_be_bytes([buf[2], buf[3]]);
    // QR bit must be set (response)
    if (flags & 0x8000) == 0 {
        return None;
    }

    let rcode = flags & 0x000F;
    // RCODE 3 is NXDOMAIN (domain does not exist) -> return empty records indicating authoritative negative
    if rcode == 3 {
        return Some(Vec::new());
    }
    // Any error other than NoError (0) is a server or format failure
    if rcode != 0 {
        return None;
    }

    let qdcount = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let ancount = u16::from_be_bytes([buf[6], buf[7]]) as usize;

    let mut offset = 12;

    // Skip question section
    for _ in 0..qdcount {
        let (_, next) = parse_dns_name(buf, offset)?;
        offset = next + 4; // Skip QTYPE (2) + QCLASS (2)
    }

    let mut records = Vec::new();

    // Parse answers
    for _ in 0..ancount {
        if offset >= buf.len() {
            break;
        }
        let (_, next) = parse_dns_name(buf, offset)?;
        offset = next;
        if offset + 10 > buf.len() {
            break;
        }

        let rtype = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        let rdlength = u16::from_be_bytes([buf[offset + 8], buf[offset + 9]]) as usize;
        offset += 10;

        if rtype == 33 && rdlength >= 6 && offset + rdlength <= buf.len() {
            let priority = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
            let weight = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]);
            let port = u16::from_be_bytes([buf[offset + 4], buf[offset + 5]]);

            if let Some((target, _)) = parse_dns_name(buf, offset + 6) {
                if !target.is_empty() && target != "." {
                    records.push(SrvRecord {
                        priority,
                        weight,
                        port,
                        target,
                    });
                }
            }
        }
        offset += rdlength;
    }

    records.sort_by_key(|r| r.priority);
    Some(records)
}

/// Discovers system DNS nameservers from `/etc/resolv.conf`, falling back to 127.0.0.53 and 1.1.1.1.
pub fn get_system_nameservers() -> Vec<SocketAddr> {
    let mut servers = Vec::new();
    if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("nameserver") {
                let mut parts = line.split_whitespace();
                parts.next();
                if let Some(ip_str) = parts.next() {
                    if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                        servers.push(SocketAddr::new(ip, 53));
                    }
                }
            }
        }
    }
    if servers.is_empty() {
        servers.push("127.0.0.53:53".parse().unwrap());
        servers.push("1.1.1.1:53".parse().unwrap());
    }
    servers
}

/// Performs an RFC 3263 DNS SRV lookup for `_sip._udp.<domain>`.
///
/// Returns `Some((target_host, port))` if an SRV record is found, or `None` on NXDOMAIN/timeout/failure.
pub async fn resolve_sip_srv(domain: &str) -> Option<(String, u16)> {
    let query_name = format!("_sip._udp.{}", domain.trim_end_matches('.'));
    let tx_id = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(12345)
        & 0xFFFF) as u16;

    let query_bytes = build_srv_query(&query_name, tx_id);
    let nameservers = get_system_nameservers();

    for ns in nameservers {
        let bind_addr = if ns.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        let sock = match UdpSocket::bind(bind_addr).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        if sock.send_to(&query_bytes, ns).await.is_err() {
            continue;
        }

        let mut buf = vec![0u8; 512];
        let recv_fut = sock.recv_from(&mut buf);

        if let Ok(Ok((len, _))) = tokio::time::timeout(Duration::from_millis(800), recv_fut).await {
            if let Some(records) = parse_srv_response(&buf[..len], tx_id) {
                if let Some(best) = records.into_iter().next() {
                    debug!(
                        port = best.port,
                        priority = best.priority,
                        "RFC 3263: Resolved DNS SRV record"
                    );
                    return Some((best.target, best.port));
                } else {
                    // Authoritative negative response (NXDOMAIN or NODATA): return early without querying backup servers
                    trace!("RFC 3263: Authoritative negative response for DNS SRV");
                    return None;
                }
            }
        }
    }

    trace!("No RFC 3263 DNS SRV records found, falling back to A/AAAA");
    None
}

/// Resolves a SIP target address per RFC 3263.
///
/// - If target is a numeric IP (IPv4 or IPv6), returns immediately without DNS queries.
/// - If target contains an explicit port, resolves directly via A/AAAA.
/// - If target is a domain name without explicit port (or default 5060), queries SRV `_sip._udp.<domain>`.
/// - Falls back to standard A/AAAA lookup if SRV fails or is absent.
pub async fn resolve_sip_target(
    target: &str,
    default_port: u16,
) -> Result<Vec<SocketAddr>, String> {
    let trimmed = target.trim();

    // 1. Direct numeric IP check (RFC 3263 Section 4.1: no DNS queries for numeric IPs)
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Ok(vec![addr]);
    }
    if let Ok(ip) = trimmed.parse::<std::net::IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, default_port)]);
    }
    if let Some(unbracketed) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if let Ok(ip) = unbracketed.parse::<std::net::IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, default_port)]);
        }
    }

    // 2. Check if target has an explicit port (e.g. "sip.provider.com:5062" or "[2001:db8::1]:5062")
    if let Some((host, port_str)) = trimmed.rsplit_once(':') {
        if !host.contains(':') || (host.starts_with('[') && host.ends_with(']')) {
            if let Ok(port) = port_str.parse::<u16>() {
                let clean_host = host.trim_matches(['[', ']']);
                return tokio::net::lookup_host((clean_host, port))
                    .await
                    .map(|iter| iter.collect())
                    .map_err(|_| "DNS resolution error: host lookup failed".to_string());
            }
        }
    }

    // 3. Domain name with default port 5060: query RFC 3263 SRV _sip._udp.<domain>
    if default_port == 5060 {
        if let Some((srv_host, srv_port)) = resolve_sip_srv(trimmed).await {
            if let Ok(addrs) = tokio::net::lookup_host((srv_host.as_str(), srv_port)).await {
                let list: Vec<SocketAddr> = addrs.collect();
                if !list.is_empty() {
                    return Ok(list);
                }
            }
        }
    }

    // 4. Fall back to standard A/AAAA on target:default_port
    match tokio::net::lookup_host((trimmed, default_port)).await {
        Ok(iter) => Ok(iter.collect()),
        Err(_) => Err("DNS resolution error: host lookup failed".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_and_parse_srv_query() {
        let query = build_srv_query("_sip._udp.example.com", 0x1234);
        assert_eq!(&query[0..2], &0x1234u16.to_be_bytes());
        assert_eq!(&query[2..4], &0x0100u16.to_be_bytes()); // RD=1
        assert_eq!(&query[4..6], &1u16.to_be_bytes()); // QDCOUNT=1
    }

    #[test]
    fn test_parse_srv_synthetic_response() {
        // Construct a synthetic DNS response for _sip._udp.example.com
        let mut resp = Vec::new();
        // Header
        resp.extend_from_slice(&0x4321u16.to_be_bytes()); // ID
        resp.extend_from_slice(&0x8180u16.to_be_bytes()); // Flags: Response, No error
        resp.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        resp.extend_from_slice(&2u16.to_be_bytes()); // ANCOUNT = 2
        resp.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
        resp.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT

        // Question: _sip._udp.example.com
        for label in ["_sip", "_udp", "example", "com"] {
            resp.push(label.len() as u8);
            resp.extend_from_slice(label.as_bytes());
        }
        resp.push(0);
        resp.extend_from_slice(&33u16.to_be_bytes()); // QTYPE
        resp.extend_from_slice(&1u16.to_be_bytes()); // QCLASS

        // Answer 1: Priority 20, Weight 0, Port 5060, Target "sip2.example.com"
        resp.extend_from_slice(&[0xC0, 0x0C]); // Pointer to question name
        resp.extend_from_slice(&33u16.to_be_bytes()); // TYPE SRV
        resp.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
        resp.extend_from_slice(&300u32.to_be_bytes()); // TTL
        // RDLENGTH = 2 (priority) + 2 (weight) + 2 (port) + target length
        let mut rdata1 = Vec::new();
        rdata1.extend_from_slice(&20u16.to_be_bytes()); // Priority 20
        rdata1.extend_from_slice(&0u16.to_be_bytes()); // Weight 0
        rdata1.extend_from_slice(&5060u16.to_be_bytes()); // Port 5060
        rdata1.push(4);
        rdata1.extend_from_slice(b"sip2");
        rdata1.extend_from_slice(&[0xC0, 0x16]); // Pointer to "example.com"
        resp.extend_from_slice(&(rdata1.len() as u16).to_be_bytes());
        resp.extend_from_slice(&rdata1);

        // Answer 2: Priority 10, Weight 0, Port 5061, Target "sip1.example.com"
        resp.extend_from_slice(&[0xC0, 0x0C]);
        resp.extend_from_slice(&33u16.to_be_bytes());
        resp.extend_from_slice(&1u16.to_be_bytes());
        resp.extend_from_slice(&300u32.to_be_bytes());
        let mut rdata2 = Vec::new();
        rdata2.extend_from_slice(&10u16.to_be_bytes()); // Priority 10 (higher priority)
        rdata2.extend_from_slice(&0u16.to_be_bytes());
        rdata2.extend_from_slice(&5061u16.to_be_bytes());
        rdata2.push(4);
        rdata2.extend_from_slice(b"sip1");
        rdata2.extend_from_slice(&[0xC0, 0x16]);
        resp.extend_from_slice(&(rdata2.len() as u16).to_be_bytes());
        resp.extend_from_slice(&rdata2);

        let parsed = parse_srv_response(&resp, 0x4321).expect("parse successful");
        assert_eq!(parsed.len(), 2);
        // Priority 10 sorted first
        assert_eq!(parsed[0].priority, 10);
        assert_eq!(parsed[0].port, 5061);
        assert_eq!(parsed[0].target, "sip1.example.com");

        // Priority 20 sorted second
        assert_eq!(parsed[1].priority, 20);
        assert_eq!(parsed[1].port, 5060);
        assert_eq!(parsed[1].target, "sip2.example.com");
    }

    #[test]
    fn test_parse_srv_nxdomain() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&0x5555u16.to_be_bytes()); // ID
        resp.extend_from_slice(&0x8183u16.to_be_bytes()); // QR=1, RCODE=3 (NXDOMAIN)
        resp.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        resp.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
        resp.extend_from_slice(&0u16.to_be_bytes());
        resp.extend_from_slice(&0u16.to_be_bytes());

        let parsed = parse_srv_response(&resp, 0x5555).expect("parse response");
        assert!(
            parsed.is_empty(),
            "NXDOMAIN returns empty records for early return"
        );
    }

    #[tokio::test]
    async fn test_resolve_sip_target_numeric_ips() {
        // IPv4 without port
        let res = resolve_sip_target("192.168.1.50", 5060).await.unwrap();
        assert_eq!(
            res,
            vec!["192.168.1.50:5060".parse::<SocketAddr>().unwrap()]
        );

        // IPv4 with explicit port
        let res = resolve_sip_target("10.0.0.1:5080", 5060).await.unwrap();
        assert_eq!(res, vec!["10.0.0.1:5080".parse::<SocketAddr>().unwrap()]);

        // IPv6 without port
        let res = resolve_sip_target("::1", 5060).await.unwrap();
        assert_eq!(res, vec!["[::1]:5060".parse::<SocketAddr>().unwrap()]);

        // IPv6 bracketed with port
        let res = resolve_sip_target("[::1]:5090", 5060).await.unwrap();
        assert_eq!(res, vec!["[::1]:5090".parse::<SocketAddr>().unwrap()]);

        // IPv6 bracketed without port
        let res = resolve_sip_target("[fe80::1]", 5060).await.unwrap();
        assert_eq!(res, vec!["[fe80::1]:5060".parse::<SocketAddr>().unwrap()]);
    }
}
