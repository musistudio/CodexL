use std::net::TcpListener;

pub async fn is_port_free(host: &str, port: u16) -> bool {
    TcpListener::bind((host, port)).is_ok()
}

pub async fn find_free_port(host: &str, start_port: u16, attempts: u16) -> Option<u16> {
    for offset in 0..attempts {
        let port = start_port + offset as u16;
        if is_port_free(host, port).await {
            return Some(port);
        }
    }
    None
}

pub async fn prepare_cdp_port(host: &str, preferred_port: u16) -> u16 {
    if is_port_free(host, preferred_port).await {
        return preferred_port;
    }
    find_free_port(host, preferred_port + 1, 100)
        .await
        .unwrap_or(preferred_port)
}
