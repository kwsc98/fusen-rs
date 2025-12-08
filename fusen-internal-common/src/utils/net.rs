use local_ip_address::Error;

pub fn get_network_ip() -> Result<String, Error> {
    local_ip_address::local_ip().map(|e| e.to_string())
}
