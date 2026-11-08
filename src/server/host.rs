//! Host header validation. Only local names are accepted, which blocks DNS rebinding.

const ALLOWED_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "host.docker.internal"];

pub fn allowed(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        // [::1]:4567
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        }
    } else if host.matches(':').count() == 1 {
        host.split_once(':').map_or(host, |(name, _)| name)
    } else {
        // Bare name, or a bare IPv6 address without a port.
        host
    };
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn hosts() {
        for ok in [
            "localhost",
            "localhost:4567",
            "LOCALHOST:4567",
            "127.0.0.1:4567",
            "[::1]:4567",
            "[::1]",
            "::1",
            "host.docker.internal:4567",
            "Host.Docker.Internal",
        ] {
            assert!(allowed(Some(ok)), "{ok}");
        }
        for ng in [
            "",
            "example.com",
            "example.com:4567",
            "localhost.example.com",
            "127.0.0.2:4567",
            "[::2]:4567",
            "[::1",
            "192.168.5.2:4567",
        ] {
            assert!(!allowed(Some(ng)), "{ng}");
        }
        assert!(!allowed(None));
    }
}
