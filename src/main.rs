use std::io::{Error, Read, Write};
use std::net::TcpStream;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let server_address = "192.168.86.138:554"; // RTSP default port is 554
    let mut stream = TcpStream::connect(server_address)?;

    send_options_request(&mut stream, server_address)?;

    Ok(())
}

fn send_options_request(stream: &mut TcpStream, url: &str) -> Result<(), Error> {
    let request = format!("OPTIONS {} RTSP/1.0\r\nCSeq: 1\r\n\r\n", url);

    let mut buffer = [0; 1024];

    stream.write(request.as_bytes())?;
    stream.read(&mut buffer)?;

    print!("{}", String::from_utf8_lossy(&buffer));

    Ok(())
}
