pub fn get_network_ip() -> String {
    local_ip_address::local_ip().unwrap().to_string()
}
