use std::net::IpAddr;

pub(super) fn host_is_loopback(host: &str) -> bool {
    host_without_port(host).is_some_and(host_part_is_loopback)
}

fn host_without_port(host: &str) -> Option<&str> {
    let trimmed = non_empty_trimmed(host)?;
    Some(
        bracketed_host(trimmed)
            .map(str::trim)
            .unwrap_or_else(|| strip_numeric_host_port(trimmed)),
    )
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn bracketed_host(host: &str) -> Option<&str> {
    host.strip_prefix('[')
        .map(|stripped| stripped.split(']').next().unwrap_or(host))
}

fn strip_numeric_host_port(host: &str) -> &str {
    host.rsplit_once(':')
        .filter(|(candidate_host, candidate_port)| {
            is_strippable_host_port(candidate_host, candidate_port)
        })
        .map(|(candidate_host, _)| candidate_host.trim())
        .unwrap_or(host)
}

fn is_strippable_host_port(candidate_host: &str, candidate_port: &str) -> bool {
    is_plain_host(candidate_host) && is_ascii_port(candidate_port)
}

fn is_plain_host(candidate_host: &str) -> bool {
    !candidate_host.contains(':') && !candidate_host.is_empty()
}

fn is_ascii_port(candidate_port: &str) -> bool {
    candidate_port.chars().all(|ch| ch.is_ascii_digit())
}

fn host_part_is_loopback(host: &str) -> bool {
    host == "localhost" || ip_addr_is_loopback(host)
}

fn ip_addr_is_loopback(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}
