use std::net::IpAddr;

pub fn get_network_ip() -> std::result::Result<IpAddr, Box<dyn std::error::Error>> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let local_ip = socket.local_addr()?.ip();
    Ok(local_ip)
}

pub fn get_ip() -> String {
    match get_network_ip() {
        Ok(ok) => ok.to_string(),
        Err(_err) => "127.0.0.1".to_string(),
    }
}
