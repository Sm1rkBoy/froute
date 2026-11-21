use std::error::Error;
use std::net::Ipv4Addr;
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
struct Route {
    network: Ipv4Addr,
    prefix: u8,
    next_hop: String,
    interface: String,
    metric: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("froute: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let output = read_system_routes()?;
    let mut routes = if cfg!(target_os = "windows") {
        parse_windows(&output)
    } else if cfg!(target_os = "macos") {
        parse_bsd_netstat(&output)
    } else {
        let routes = parse_linux_ip(&output);
        if routes.is_empty() {
            parse_linux_netstat(&output)
        } else {
            routes
        }
    };

    if routes.is_empty() {
        return Err("未能从系统路由表中识别出 IPv4 路由".into());
    }

    sort_routes(&mut routes);

    let headers = ["掩码长度", "网络号", "出接口", "下一跳", "度量"];
    let rows: Vec<[String; 5]> = routes
        .into_iter()
        .map(|route| {
            [
                route.prefix.to_string(),
                route.network.to_string(),
                route.interface,
                route.next_hop,
                route.metric,
            ]
        })
        .collect();
    let mut widths: Vec<usize> = headers.iter().map(|value| display_width(value)).collect();
    for row in &rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(display_width(value));
        }
    }

    print_aligned_row(&headers, &widths);
    for row in &rows {
        print_aligned_row(row, &widths);
    }
    Ok(())
}

fn sort_routes(routes: &mut [Route]) {
    routes.sort_by(|left, right| {
        u32::from(left.network)
            .cmp(&u32::from(right.network))
            .then_with(|| right.prefix.cmp(&left.prefix))
            .then_with(|| left.interface.cmp(&right.interface))
            .then_with(|| left.next_hop.cmp(&right.next_hop))
    });
}

fn display_width(value: &str) -> usize {
    value
        .chars()
        .map(|character| {
            if matches!(character as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60) {
                2
            } else {
                1
            }
        })
        .sum()
}

fn print_aligned_row<const N: usize>(fields: &[impl AsRef<str>; N], widths: &[usize]) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            print!("  ");
        }
        let value = field.as_ref();
        print!("{value}");
        if index + 1 < N {
            let padding = widths[index].saturating_sub(display_width(value));
            print!("{}", " ".repeat(padding));
        }
    }
    println!();
}

fn read_system_routes() -> Result<String, Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let commands: &[(&str, &[&str])] = &[("route", &["print", "-4"])];
    #[cfg(target_os = "macos")]
    let commands: &[(&str, &[&str])] = &[("netstat", &["-rn"])];
    #[cfg(target_os = "linux")]
    let commands: &[(&str, &[&str])] = &[
        ("ip", &["-4", "route", "show"]),
        ("netstat", &["-rn", "-4"]),
    ];
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let commands: &[(&str, &[&str])] = &[];

    for (program, args) in commands {
        if let Ok(output) = Command::new(program).args(*args).output() {
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
    }
    Err("无法读取系统路由表；请确认系统支持，且 ip/netstat/route 命令可用".into())
}

fn parse_linux_ip(text: &str) -> Vec<Route> {
    let mut routes = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.is_empty() {
            continue;
        }
        let destination = fields[0];
        let Ok((network, prefix)) = parse_destination(destination) else {
            continue;
        };
        let mut gateway = "直连".to_owned();
        let mut interface = "-".to_owned();
        let mut metric = "0".to_owned();
        let mut index = 1;
        while index + 1 < fields.len() {
            match fields[index] {
                "via" => gateway = fields[index + 1].to_owned(),
                "dev" => interface = fields[index + 1].to_owned(),
                "metric" => metric = fields[index + 1].to_owned(),
                _ => {}
            }
            index += 1;
        }
        routes.push(Route {
            network,
            prefix,
            next_hop: gateway,
            interface,
            metric,
        });
    }
    routes
}

fn parse_linux_netstat(text: &str) -> Vec<Route> {
    let mut routes = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 || fields[0] == "Destination" || fields[0] == "Kernel" {
            continue;
        }
        let Ok((network, prefix)) = parse_destination(fields[0]) else {
            continue;
        };
        if fields[1].parse::<Ipv4Addr>().is_err() {
            continue;
        }
        let gateway = if fields[1] == "0.0.0.0" {
            "直连"
        } else {
            fields[1]
        };
        routes.push(Route {
            network,
            prefix,
            next_hop: gateway.to_owned(),
            interface: fields[7].to_owned(),
            metric: fields[4].to_owned(),
        });
    }
    routes
}

fn parse_windows(text: &str) -> Vec<Route> {
    let mut routes = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Network Destination") {
            in_table = true;
            continue;
        }
        if !in_table || trimmed.is_empty() {
            continue;
        }
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let (Ok(destination), Ok(mask)) =
            (fields[0].parse::<Ipv4Addr>(), fields[1].parse::<Ipv4Addr>())
        else {
            continue;
        };
        let Ok(prefix) = mask_to_prefix(mask) else {
            continue;
        };
        routes.push(Route {
            network: mask_address(destination, mask),
            prefix,
            next_hop: if fields[2] == "On-link" {
                "直连".to_owned()
            } else {
                fields[2].to_owned()
            },
            interface: fields[3].to_owned(),
            metric: fields[4].to_owned(),
        });
    }
    routes
}

fn parse_bsd_netstat(text: &str) -> Vec<Route> {
    let mut routes = Vec::new();
    let mut in_ipv6_table = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Internet6:") {
            in_ipv6_table = true;
            continue;
        }
        if trimmed.starts_with("Internet:") {
            in_ipv6_table = false;
            continue;
        }
        if in_ipv6_table {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[0] == "Destination" {
            continue;
        }
        let Ok((network, prefix)) = parse_bsd_destination(fields[0]) else {
            continue;
        };
        // BSD netstat has no portable metric column, so report it as unknown.
        // `link#N` means an on-link route; a MAC address is a neighbor entry,
        // not an IPv4 next hop, so preserve it rather than calling it direct.
        let gateway = if fields[1].parse::<Ipv4Addr>().is_ok() {
            fields[1].to_owned()
        } else if fields[1].starts_with("link#") {
            "直连".to_owned()
        } else {
            fields[1].to_owned()
        };
        let interface = fields[3].to_owned();
        routes.push(Route {
            network,
            prefix,
            next_hop: gateway,
            interface,
            metric: "-".to_owned(),
        });
    }
    routes
}

fn parse_bsd_destination(value: &str) -> Result<(Ipv4Addr, u8), Box<dyn Error>> {
    if value == "default" {
        return Ok((Ipv4Addr::UNSPECIFIED, 0));
    }
    let (address, explicit_prefix) = match value.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix.parse::<u8>()?)),
        None => (value, None),
    };
    if let Ok(address) = address.parse::<Ipv4Addr>() {
        let prefix = explicit_prefix.unwrap_or(32);
        if prefix > 32 {
            return Err("IPv4 前缀长度超出范围".into());
        }
        let mask = prefix_to_mask(prefix);
        return Ok((mask_address(address, mask), prefix));
    }

    let octets: Vec<&str> = address.split('.').collect();
    if octets.is_empty() || octets.len() > 4 || octets.iter().any(|part| part.is_empty()) {
        return Err("无效的 BSD 路由目标".into());
    }
    let mut bytes = [0u8; 4];
    for (index, octet) in octets.iter().enumerate() {
        bytes[index] = octet.parse()?;
    }
    let prefix = explicit_prefix.unwrap_or((octets.len() * 8) as u8);
    if prefix > 32 {
        return Err("IPv4 前缀长度超出范围".into());
    }
    let mask = prefix_to_mask(prefix);
    Ok((mask_address(Ipv4Addr::from(bytes), mask), prefix))
}

fn parse_destination(value: &str) -> Result<(Ipv4Addr, u8), Box<dyn Error>> {
    if value == "default" || value == "0.0.0.0/0" {
        return Ok((Ipv4Addr::UNSPECIFIED, 0));
    }
    if let Some((address, prefix)) = value.split_once('/') {
        let address = address.parse::<Ipv4Addr>()?;
        let prefix = prefix.parse::<u8>()?;
        if prefix > 32 {
            return Err("IPv4 前缀长度超出范围".into());
        }
        return Ok((mask_address(address, prefix_to_mask(prefix)), prefix));
    }
    let address = value.parse::<Ipv4Addr>()?;
    Ok((address, 32))
}

fn prefix_to_mask(prefix: u8) -> Ipv4Addr {
    let bits = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Ipv4Addr::from(bits)
}

fn mask_to_prefix(mask: Ipv4Addr) -> Result<u8, Box<dyn Error>> {
    let bits = u32::from(mask);
    let prefix = bits.count_ones() as u8;
    if prefix_to_mask(prefix) != mask {
        return Err("无效的非连续子网掩码".into());
    }
    Ok(prefix)
}

fn mask_address(address: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(address) & u32::from(mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_routes_and_normalizes_network() {
        let routes = parse_linux_ip("default via 192.168.1.1 dev wlan0 metric 600\n192.168.1.37/24 dev wlan0 proto kernel scope link src 192.168.1.37");
        assert_eq!(routes.len(), 2);
        assert_eq!(
            routes[0],
            Route {
                network: "0.0.0.0".parse().unwrap(),
                prefix: 0,
                next_hop: "192.168.1.1".into(),
                interface: "wlan0".into(),
                metric: "600".into()
            }
        );
        assert_eq!(
            routes[1].network,
            "192.168.1.0".parse::<Ipv4Addr>().unwrap()
        );
        assert_eq!(routes[1].prefix, 24);
    }

    #[test]
    fn parses_windows_route_table() {
        let routes = parse_windows("Network Destination        Netmask          Gateway       Interface  Metric\n          0.0.0.0          0.0.0.0      10.0.0.1       10.0.0.2     25\n");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].next_hop, "10.0.0.1");
        assert_eq!(routes[0].metric, "25");
    }

    #[test]
    fn parses_bsd_abbreviated_networks() {
        let (network, prefix) = parse_bsd_destination("192.168.10/24").unwrap();
        assert_eq!(network, "192.168.10.0".parse::<Ipv4Addr>().unwrap());
        assert_eq!(prefix, 24);
    }

    #[test]
    fn parses_bsd_link_gateway_and_interface() {
        let routes = parse_bsd_netstat(
            "Destination Gateway Flags Netif Expire\n192.168.1 link#6 UCS en0\ndefault 192.168.1.1 UGSc en0\n192.168.1.4 16:33:29:24:7d:d9 UHLWI lo0",
        );
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].next_hop, "直连");
        assert_eq!(routes[0].interface, "en0");
        assert_eq!(routes[1].next_hop, "192.168.1.1");
        assert_eq!(routes[1].interface, "en0");
        assert_eq!(routes[1].metric, "-");
        assert_eq!(routes[2].next_hop, "16:33:29:24:7d:d9");
        assert_eq!(routes[2].interface, "lo0");
    }

    #[test]
    fn parses_bsd_ipv4_table_and_skips_ipv6_table() {
        let routes = parse_bsd_netstat(
            "Routing tables\nInternet:\nDestination Gateway Flags Netif Expire\ndefault 192.168.1.1 UGSc en0\n192.168.1.5 a2:ec:21:71:58:31 UHLWI en1\nInternet6:\nDestination Gateway Flags Netif Expire\ndefault fe80::a:1%en1 UG en1",
        );
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].next_hop, "192.168.1.1");
        assert_eq!(routes[1].next_hop, "a2:ec:21:71:58:31");
    }

    #[test]
    fn sorts_routes_by_network_and_groups_similar_networks() {
        let mut routes = vec![
            Route {
                network: "192.168.1.0".parse().unwrap(),
                prefix: 24,
                next_hop: "直连".into(),
                interface: "en0".into(),
                metric: "-".into(),
            },
            Route {
                network: "172.21.95.0".parse().unwrap(),
                prefix: 24,
                next_hop: "直连".into(),
                interface: "vpn0".into(),
                metric: "-".into(),
            },
            Route {
                network: "172.20.0.0".parse().unwrap(),
                prefix: 14,
                next_hop: "直连".into(),
                interface: "eth0".into(),
                metric: "0".into(),
            },
            Route {
                network: "172.20.0.0".parse().unwrap(),
                prefix: 16,
                next_hop: "直连".into(),
                interface: "eth1".into(),
                metric: "0".into(),
            },
        ];

        sort_routes(&mut routes);

        assert_eq!(routes[0].network, "172.20.0.0".parse::<Ipv4Addr>().unwrap());
        assert_eq!(routes[0].prefix, 16);
        assert_eq!(routes[1].network, "172.20.0.0".parse::<Ipv4Addr>().unwrap());
        assert_eq!(routes[1].prefix, 14);
        assert_eq!(
            routes[2].network,
            "172.21.95.0".parse::<Ipv4Addr>().unwrap()
        );
        assert_eq!(
            routes[3].network,
            "192.168.1.0".parse::<Ipv4Addr>().unwrap()
        );
    }

    #[test]
    fn rejects_non_contiguous_netmask() {
        assert!(mask_to_prefix("255.0.255.0".parse().unwrap()).is_err());
    }
}
