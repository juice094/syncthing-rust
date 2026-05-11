
    #[test]
    fn test_stun_client_default() {
        let client = StunClient::default();
        assert!(!client.servers.is_empty());
        assert_eq!(client.timeout, STUN_TIMEOUT);
    }

    #[test]
    fn test_stun_client_with_servers() {
        let servers = vec!["stun.example.com:3478".to_string()];
        let client = StunClient::with_servers(servers.clone());
        assert_eq!(client.servers, servers);
    }

    #[test]
    fn test_is_public_address() {
        // IPv4 公网地址
        assert!(StunClient::is_public_address(
            &"8.8.8.8:1234".parse().unwrap()
        ));
        assert!(StunClient::is_public_address(
            &"1.2.3.4:1234".parse().unwrap()
        ));

        // IPv4 私有地址
        assert!(!StunClient::is_public_address(
            &"10.0.0.1:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"192.168.1.1:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"172.16.0.1:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"127.0.0.1:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"169.254.1.1:1234".parse().unwrap()
        ));

        // IPv6 公网地址
        assert!(StunClient::is_public_address(
            &"[2001:db8::1]:1234".parse().unwrap()
        ));

        // IPv6 私有/特殊地址
        assert!(!StunClient::is_public_address(
            &"[::1]:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"[fe80::1]:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"[fc00::1]:1234".parse().unwrap()
        ));
        assert!(!StunClient::is_public_address(
            &"[ff02::1]:1234".parse().unwrap()
        ));
    }

    #[test]
    fn test_stun_request_building() {
        let client = StunClient::new();
        let request = client.build_binding_request().unwrap();

        assert!(is_stun_packet(&request));
        assert_eq!(&request[0..2], &BINDING_REQUEST);
        assert_eq!(&request[4..8], &MAGIC_COOKIE);
        // 事务 ID 在 8..20
        let tx_id = &request[8..20];
        assert_eq!(tx_id.len(), 12);
    }

    #[test]
    fn test_parse_response_ipv4_xor_mapped() {
        let tx_id = [1u8; 12];
        let public_addr = SocketAddr::from(([192, 0, 2, 1], 54321));
        let response = build_test_response(tx_id, public_addr, true);

        let (parsed_tx_id, parsed_addr) = parse_response(&response).unwrap();
        assert_eq!(parsed_tx_id, tx_id);
        assert_eq!(parsed_addr, public_addr);
    }

    #[test]
    fn test_parse_response_ipv4_mapped_fallback() {
        let tx_id = [2u8; 12];
        let public_addr = SocketAddr::from(([198, 51, 100, 5], 12345));
        let response = build_test_response(tx_id, public_addr, false);

        let (parsed_tx_id, parsed_addr) = parse_response(&response).unwrap();
        assert_eq!(parsed_tx_id, tx_id);
        assert_eq!(parsed_addr, public_addr);
    }

    #[test]
    fn test_parse_response_ipv6_xor_mapped() {
        let tx_id = [3u8; 12];
        let public_addr = SocketAddr::from(([0x2001, 0xdb8, 0, 0, 0, 0, 0, 1], 12345));
        let response = build_test_response(tx_id, public_addr, true);

        let (parsed_tx_id, parsed_addr) = parse_response(&response).unwrap();
        assert_eq!(parsed_tx_id, tx_id);
        assert_eq!(parsed_addr, public_addr);
    }

    #[test]
    fn test_parse_response_not_stun() {
        let data = b"not a stun packet";
        assert!(parse_response(data).is_err());
    }

    #[test]
    fn test_parse_response_missing_address() {
        let tx_id = [4u8; 12];
        let mut response = Vec::with_capacity(HEADER_LEN);
        response.extend_from_slice(&BINDING_SUCCESS_RESPONSE);
        response.extend_from_slice(&0u16.to_be_bytes()); // attrs len = 0
        response.extend_from_slice(&MAGIC_COOKIE);
        response.extend_from_slice(&tx_id);
        assert!(parse_response(&response).is_err());
    }

    #[tokio::test]
    async fn test_query_mock_server() {
        let bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let socket = UdpSocket::bind(bind_addr).await.unwrap();
        let server_addr = socket.local_addr().unwrap();

        let expected_addr = SocketAddr::from(([203, 0, 113, 7], 9876));

        let server_handle = tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (len, from) = socket.recv_from(&mut buf).await.unwrap();
            let request = &buf[..len];
            assert!(is_stun_packet(request));
            let tx_id = {
                let mut id = [0u8; 12];
                id.copy_from_slice(&request[8..20]);
                id
            };
            let response = build_test_response(tx_id, expected_addr, true);
            socket.send_to(&response, from).await.unwrap();
        });

        let result = query(&server_addr.to_string(), Duration::from_secs(2)).await;
        server_handle.await.unwrap();
        assert_eq!(result.unwrap(), expected_addr);
    }

    #[tokio::test]
    #[ignore = "requires external network"]
    async fn test_query_public_stun_server() {
        let addr = query("stun.l.google.com:19302", Duration::from_secs(5))
            .await
            .unwrap();
        assert!(StunClient::is_public_address(&addr));
    }

    #[test]
    fn test_nat_type_helpers() {
        assert!(NatType::Open.is_p2p_feasible());
        assert!(NatType::Restricted.is_p2p_feasible());
        assert!(!NatType::Symmetric.is_p2p_feasible());
        assert!(!NatType::Blocked.is_p2p_feasible());
        assert!(!NatType::Unknown.is_p2p_feasible());

        assert!(!NatType::Open.needs_relay());
        assert!(!NatType::Restricted.needs_relay());
        assert!(NatType::Symmetric.needs_relay());
        assert!(NatType::Blocked.needs_relay());
        assert!(NatType::Unknown.needs_relay());
    }

    #[tokio::test]
    async fn test_detect_nat_type_mock_open() {
        // Simulate Cone NAT: both servers return the same mapped address
        let addr_a = SocketAddr::from(([127, 0, 0, 1], 0));
        let socket_a = UdpSocket::bind(addr_a).await.unwrap();
        let server_a = socket_a.local_addr().unwrap();

        let addr_b = SocketAddr::from(([127, 0, 0, 1], 0));
        let socket_b = UdpSocket::bind(addr_b).await.unwrap();
        let server_b = socket_b.local_addr().unwrap();

        let mapped = SocketAddr::from(([203, 0, 113, 7], 9876));

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (len, from) = socket_a.recv_from(&mut buf).await.unwrap();
            let tx_id = extract_tx_id(&buf[..len]);
            socket_a
                .send_to(&build_test_response(tx_id, mapped, true), from)
                .await
                .unwrap();
        });

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (len, from) = socket_b.recv_from(&mut buf).await.unwrap();
            let tx_id = extract_tx_id(&buf[..len]);
            socket_b
                .send_to(&build_test_response(tx_id, mapped, true), from)
                .await
                .unwrap();
        });

        let client = StunClient::with_servers(vec![server_a.to_string(), server_b.to_string()])
            .with_timeout(Duration::from_secs(2));

        let (nat_type, pub_addr) = client.detect_nat_type().await.unwrap();
        assert_eq!(nat_type, NatType::Open);
        assert_eq!(pub_addr, Some(mapped));
    }

    #[tokio::test]
    async fn test_detect_nat_type_mock_symmetric() {
        // Simulate Symmetric NAT: servers return different mapped addresses
        let addr_a = SocketAddr::from(([127, 0, 0, 1], 0));
        let socket_a = UdpSocket::bind(addr_a).await.unwrap();
        let server_a = socket_a.local_addr().unwrap();

        let addr_b = SocketAddr::from(([127, 0, 0, 1], 0));
        let socket_b = UdpSocket::bind(addr_b).await.unwrap();
        let server_b = socket_b.local_addr().unwrap();

        let mapped_a = SocketAddr::from(([203, 0, 113, 7], 9876));
        let mapped_b = SocketAddr::from(([203, 0, 113, 8], 1234));

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (len, from) = socket_a.recv_from(&mut buf).await.unwrap();
            let tx_id = extract_tx_id(&buf[..len]);
            socket_a
                .send_to(&build_test_response(tx_id, mapped_a, true), from)
                .await
                .unwrap();
        });

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (len, from) = socket_b.recv_from(&mut buf).await.unwrap();
            let tx_id = extract_tx_id(&buf[..len]);
            socket_b
                .send_to(&build_test_response(tx_id, mapped_b, true), from)
                .await
                .unwrap();
        });

        let client = StunClient::with_servers(vec![server_a.to_string(), server_b.to_string()])
            .with_timeout(Duration::from_secs(2));

        let (nat_type, pub_addr) = client.detect_nat_type().await.unwrap();
        assert_eq!(nat_type, NatType::Symmetric);
        assert_eq!(pub_addr, Some(mapped_a));
    }

    #[tokio::test]
    async fn test_detect_nat_type_mock_blocked() {
        // Simulate blocked UDP: server does not respond
        let addr_a = SocketAddr::from(([127, 0, 0, 1], 0));
        let socket_a = UdpSocket::bind(addr_a).await.unwrap();
        let server_a = socket_a.local_addr().unwrap();

        // Bind server B but intentionally drop all packets
        let addr_b = SocketAddr::from(([127, 0, 0, 1], 0));
        let _socket_b = UdpSocket::bind(addr_b).await.unwrap();
        let server_b = _socket_b.local_addr().unwrap();

        // Server A: no response (just receive and drop)
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = socket_a.recv_from(&mut buf).await;
            // Intentionally do not respond
        });

        let client = StunClient::with_servers(vec![server_a.to_string(), server_b.to_string()])
            .with_timeout(Duration::from_millis(500));

        let (nat_type, pub_addr) = client.detect_nat_type().await.unwrap();
        assert_eq!(nat_type, NatType::Blocked);
        assert_eq!(pub_addr, None);
    }

    fn extract_tx_id(data: &[u8]) -> TxId {
        let mut id = TxId::default();
        id.copy_from_slice(&data[8..20]);
        id
    }

    /// 辅助函数：构建测试用的 STUN Success Response
    fn build_test_response(tx_id: TxId, addr: SocketAddr, use_xor: bool) -> Vec<u8> {
        let (fam, ip_bytes): (u8, Vec<u8>) = match addr.ip() {
            IpAddr::V4(ip) => (0x01, ip.octets().to_vec()),
            IpAddr::V6(ip) => (0x02, ip.octets().to_vec()),
        };

        let attr_len = 4 + ip_bytes.len();
        let attrs_len = 4 + attr_len;
        let mut b = Vec::with_capacity(HEADER_LEN + attrs_len);

        // Header
        b.extend_from_slice(&BINDING_SUCCESS_RESPONSE);
        b.extend_from_slice(&(attrs_len as u16).to_be_bytes());
        b.extend_from_slice(&MAGIC_COOKIE);
        b.extend_from_slice(&tx_id);

        if use_xor {
            // XOR-MAPPED-ADDRESS
            b.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
            b.extend_from_slice(&(attr_len as u16).to_be_bytes());
            b.push(0);
            b.push(fam);
            let xor_port = addr.port() ^ 0x2112;
            b.extend_from_slice(&xor_port.to_be_bytes());
            for (i, &o) in ip_bytes.iter().enumerate() {
                if i < MAGIC_COOKIE.len() {
                    b.push(o ^ MAGIC_COOKIE[i]);
                } else {
                    b.push(o ^ tx_id[i - MAGIC_COOKIE.len()]);
                }
            }
        } else {
            // MAPPED-ADDRESS
            b.extend_from_slice(&ATTR_MAPPED_ADDRESS.to_be_bytes());
            b.extend_from_slice(&(attr_len as u16).to_be_bytes());
            b.push(0);
            b.push(fam);
            b.extend_from_slice(&addr.port().to_be_bytes());
            b.extend_from_slice(&ip_bytes);
        }

        b
    }
