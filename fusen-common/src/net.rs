pub fn get_network_ip() -> std::result::Result<String, Box<dyn std::error::Error>> {
    Ok(local_ip_address::local_ip()?.to_string())
}

pub fn get_ip() -> String {
    match get_network_ip() {
        Ok(ok) => ok,
        Err(_err) => "127.0.0.1".to_string(),
    }
}

pub fn get_path(mut ip: String, port: Option<&str>) -> String {
    if let Some(port) = port {
        ip.push(':');
        ip.push_str(port);
    }
    ip
}
