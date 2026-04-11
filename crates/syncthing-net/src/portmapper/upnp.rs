//! UPnP IGD 端口映射实现
//!
//! 参考来源: Tailscale net/portmapper/upnp.go

use std::net::SocketAddr;
use std::time::Duration;

use igd::aio::search_gateway;
use igd::{PortMappingProtocol, SearchOptions};
use syncthing_core::{Result, SyncthingError};

/// UPnP 发现端口
pub const UPNP_DISCOVERY_PORT: u16 = 1900;

/// SSDP 多播地址
pub const SSDP_MULTICAST_ADDR: &str = "239.255.255.250:1900";

/// SSDP M-SEARCH 请求包（查找 InternetGatewayDevice）
pub const SSDP_MSEARCH_IGD: &str = "M-SEARCH * HTTP/1.1\r\n\
HOST: 239.255.255.250:1900\r\n\
ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\
MAN: \"ssdp:discover\"\r\n\
MX: 2\r\n\r\n";

/// SSDP M-SEARCH 请求包（ssdp:all）
pub const SSDP_MSEARCH_ALL: &str = "M-SEARCH * HTTP/1.1\r\n\
HOST: 239.255.255.250:1900\r\n\
ST: ssdp:all\r\n\
MAN: \"ssdp:discover\"\r\n\
MX: 2\r\n\r\n";

/// UPnP 发现响应
#[derive(Debug, Clone)]
pub struct UpnpDiscoResponse {
    pub location: String,
    pub server: String,
    pub usn: String,
}

/// UPnP 映射状态
#[derive(Debug, Clone)]
pub struct UpnpMappingState {
    pub external_port: u16,
    pub protocol: String,
}

/// 解析 SSDP 发现响应
pub fn parse_upnp_disco_response(body: &[u8]) -> Option<UpnpDiscoResponse> {
    let text = String::from_utf8_lossy(body);
    let upper = text.to_uppercase();
    if !upper.starts_with("HTTP/1.1 200 OK") && !upper.starts_with("HTTP/1.0 200 OK") {
        return None;
    }

    let mut location = String::new();
    let mut server = String::new();
    let mut usn = String::new();

    for line in text.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("location:") {
            location = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
        } else if lower.starts_with("server:") {
            server = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
        } else if lower.starts_with("usn:") {
            usn = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
        }
    }

    if location.is_empty() {
        return None;
    }

    Some(UpnpDiscoResponse { location, server, usn })
}

/// 构建 AddPortMapping SOAP 请求体
pub fn build_add_port_mapping_soap(
    external_port: u16,
    internal_port: u16,
    internal_client: &str,
    protocol: &str,
    duration_sec: u32,
    description: &str,
) -> String {
    format!(
        r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:AddPortMapping xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1">
      <NewRemoteHost></NewRemoteHost>
      <NewExternalPort>{external_port}</NewExternalPort>
      <NewProtocol>{protocol}</NewProtocol>
      <NewInternalPort>{internal_port}</NewInternalPort>
      <NewInternalClient>{internal_client}</NewInternalClient>
      <NewEnabled>1</NewEnabled>
      <NewPortMappingDescription>{description}</NewPortMappingDescription>
      <NewLeaseDuration>{duration_sec}</NewLeaseDuration>
    </u:AddPortMapping>
  </s:Body>
</s:Envelope>"#
    )
}

/// 构建 DeletePortMapping SOAP 请求体
pub fn build_delete_port_mapping_soap(external_port: u16, protocol: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:DeletePortMapping xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1">
      <NewRemoteHost></NewRemoteHost>
      <NewExternalPort>{external_port}</NewExternalPort>
      <NewProtocol>{protocol}</NewProtocol>
    </u:DeletePortMapping>
  </s:Body>
</s:Envelope>"#
    )
}

/// 分配 UPnP 端口映射
///
/// 当前使用 igd 库进行网关发现和端口映射。
/// TODO: 未来可替换为原始 SSDP / SOAP 实现。
pub(crate) async fn allocate_port(local_addr: SocketAddr, local_port: u16) -> Result<(SocketAddr, UpnpMappingState)> {
    let options = SearchOptions {
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };

    let gateway = search_gateway(options)
        .await
        .map_err(|e| SyncthingError::connection(format!("UPnP gateway discovery failed: {}", e)))?;

    let local_v4 = match local_addr {
        SocketAddr::V4(v4) => v4,
        SocketAddr::V6(_) => {
            return Err(SyncthingError::config("IPv6 not supported for UPnP"));
        }
    };

    let external_port = local_port;
    let duration = super::PORT_MAP_LIFETIME_SEC;

    gateway
        .add_port(
            PortMappingProtocol::UDP,
            external_port,
            local_v4,
            duration,
            "syncthing-portmapper",
        )
        .await
        .map_err(|e| SyncthingError::connection(format!("UPnP AddPortMapping failed: {}", e)))?;

    let external_ip = gateway
        .get_external_ip()
        .await
        .map_err(|e| SyncthingError::connection(format!("UPnP GetExternalIPAddress failed: {}", e)))?;

    let external = SocketAddr::from((std::net::IpAddr::from(external_ip), external_port));
    let state = UpnpMappingState {
        external_port,
        protocol: "UDP".to_string(),
    };

    Ok((external, state))
}

/// 释放 UPnP 端口映射
pub(crate) async fn release_mapping(state: &UpnpMappingState) -> Result<()> {
    let options = SearchOptions {
        timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };

    let gateway = match search_gateway(options).await {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!("UPnP gateway not found for release: {}", e);
            return Ok(());
        }
    };

    let protocol = if state.protocol == "TCP" {
        PortMappingProtocol::TCP
    } else {
        PortMappingProtocol::UDP
    };

    gateway
        .remove_port(protocol, state.external_port)
        .await
        .map_err(|e| SyncthingError::connection(format!("UPnP DeletePortMapping failed: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_upnp_disco_response() {
        let body = b"HTTP/1.1 200 OK\r\n\
CACHE-CONTROL: max-age=120\r\n\
LOCATION: http://192.168.1.1:5000/rootDesc.xml\r\n\
SERVER: MiniUPnPd/2.2.1\r\n\
ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\
USN: uuid:abc::urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";

        let res = parse_upnp_disco_response(body).unwrap();
        assert_eq!(res.location, "http://192.168.1.1:5000/rootDesc.xml");
        assert_eq!(res.server, "MiniUPnPd/2.2.1");
        assert_eq!(res.usn, "uuid:abc::urn:schemas-upnp-org:device:InternetGatewayDevice:1");
    }

    #[test]
    fn test_parse_upnp_disco_response_missing_location() {
        let body = b"HTTP/1.1 200 OK\r\nSERVER: Test\r\n\r\n";
        assert!(parse_upnp_disco_response(body).is_none());
    }

    #[test]
    fn test_parse_upnp_disco_response_not_http_200() {
        let body = b"NOTIFY * HTTP/1.1\r\nLOCATION: http://x\r\n\r\n";
        assert!(parse_upnp_disco_response(body).is_none());
    }

    #[test]
    fn test_build_add_port_mapping_soap() {
        let soap = build_add_port_mapping_soap(
            22000,
            22000,
            "192.168.1.100",
            "UDP",
            7200,
            "syncthing-portmapper",
        );
        assert!(soap.contains("<u:AddPortMapping"));
        assert!(soap.contains("<NewExternalPort>22000</NewExternalPort>"));
        assert!(soap.contains("<NewInternalPort>22000</NewInternalPort>"));
        assert!(soap.contains("<NewInternalClient>192.168.1.100</NewInternalClient>"));
        assert!(soap.contains("<NewProtocol>UDP</NewProtocol>"));
        assert!(soap.contains("<NewLeaseDuration>7200</NewLeaseDuration>"));
        assert!(soap.contains("<NewPortMappingDescription>syncthing-portmapper</NewPortMappingDescription>"));
    }

    #[test]
    fn test_build_delete_port_mapping_soap() {
        let soap = build_delete_port_mapping_soap(22000, "UDP");
        assert!(soap.contains("<u:DeletePortMapping"));
        assert!(soap.contains("<NewExternalPort>22000</NewExternalPort>"));
        assert!(soap.contains("<NewProtocol>UDP</NewProtocol>"));
    }
}
