use anyhow::{Error, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use tokio::net::UdpSocket;

#[derive(Debug)]
struct Session {
    cseq: u32,
    server_addr: String,
    stream: TcpStream,
    transport: String,
    track: String,
    name: String,
}

impl Session {
    fn new(server_addr: String, stream: TcpStream) -> Self {
        Session {
            server_addr,
            stream,
            transport: String::new(),
            track: String::new(),
            name: String::new(),
            cseq: 1,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let rtsp_addr = "192.168.86.138:554";

    let mut session = Session::new(rtsp_addr.to_string(), TcpStream::connect(rtsp_addr)?);

    let response = send_basic_rtsp_request(&mut session, "OPTIONS").await?;
    println!("OPTIONS: \n{response}");

    let response = send_basic_rtsp_request(&mut session, "DESCRIBE").await?;

    // Response from a DESCRIBE method will also have an SDP data
    // SDP data begins after \r\n\r\n
    let (headers, sdp) = response.split_once("\r\n\r\n").unwrap();

    println!("DESCRIBE headers: {headers}");
    println!("DESCRIBE session: {sdp}");

    // Indicate in the Transport heading whether you want TCP/UDP
    // With this camera it seems when TCP is chosen, then the
    // server will NOT respond with a port number. I guess this
    // means that it uses existing TCP connection to send RTP?
    // When UDP is chosen, a port is provided in response. With
    // this camera (Topodome) choosing UDP provided a port in
    // the response at 6600.
    session.transport = "Transport: RTP/AVP/UDP;unicast;client_port=4588-4589\r\n".to_string();
    session.track = "/trackID=0".to_string();

    let response = send_basic_rtsp_request(&mut session, "SETUP").await?;
    println!("SETUP: \n{response}");

    // I think you need to append the token received in SETUP
    // response here. With my test camera, it was not necessary
    // and without the token, I still received 200 OK
    session.name = "Session: ".to_string();

    let response = send_basic_rtsp_request(&mut session, "PLAY").await?;
    println!("PLAY: \n{response}");

    if (&response).contains("200 OK") {
        // Bind to my client UDP port which is provided in DESCRIBE method
        // in the 'Transport' header
        let udp_stream = UdpSocket::bind("0.0.0.0:4588").await?;

        // Connect to the RTP camera server using IP and port
        // provided in SETUP response
        // In the RTP specs, the RTCP server should be
        // port 6601 and will always need to be
        // a different port
        udp_stream.connect("192.168.86.138:6600").await?;

        // Set buffer to large enough to handle RTP packets
        // in my Wireshark analysis for this camera they
        // tended be a bit more than 1024
        let mut buf = [0u8; 2048];
        loop {
            let len = udp_stream.recv(&mut buf).await?;
            println!("{:?} bytes received", len);
        }
    }

    Ok(())
}

async fn send_basic_rtsp_request(sess: &mut Session, method: &str) -> Result<String, Error> {
    let request = format!(
        "{} {}{} RTSP/1.0\r\nCSeq: {}\r\n{}\r\n{}\r\n",
        method, sess.server_addr, sess.track, sess.cseq, sess.transport, sess.name,
    );

    let mut buffer = [0; 1024];

    sess.stream.write(request.as_bytes())?;
    sess.stream.read(&mut buffer)?;
    sess.cseq += 1;

    let response = (*String::from_utf8_lossy(&buffer)).to_string();

    Ok(response)
}
