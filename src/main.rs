use std::io::{Error, Read, Write};
use std::net::TcpStream;

#[derive(Debug)]
struct Session {
    cseq: u32,
    server_addr: String,
    stream: TcpStream,
    transport: String,
    track: String,
}

impl Session {
    fn new(server_addr: String, stream: TcpStream) -> Self {
        Session {
            server_addr,
            stream,
            transport: String::new(),
            track: String::new(),
            cseq: 1,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let rtsp_addr = "192.168.86.138:554";

    let mut session = Session::new(rtsp_addr.to_string(), TcpStream::connect(rtsp_addr)?);

    let response = send_basic_rtsp_request(&mut session, "OPTIONS").await?;
    println!("OPTIONS: \n{response}");

    let response = send_basic_rtsp_request(&mut session, "DESCRIBE").await?;
    println!("DESCRIBE: \n{response}");

    session.transport = "Transport: RTP/AVP;unicast;client_port=4588-4589\r\n".to_string();
    session.track = "trackID=0".to_string();

    let response = send_basic_rtsp_request(&mut session, "SETUP").await?;
    println!("SETUP: \n{response}");

    Ok(())
}

async fn send_basic_rtsp_request(sess: &mut Session, method: &str) -> Result<String, Error> {
    let request = format!(
        "{} {}{} RTSP/1.0\r\nCSeq: {}\r\n{}\r\n",
        method, sess.server_addr, sess.track, sess.cseq, sess.transport
    );

    let mut buffer = [0; 1024];

    sess.stream.write(request.as_bytes())?;
    sess.stream.read(&mut buffer)?;
    sess.cseq += 1;

    let response = (*String::from_utf8_lossy(&buffer)).to_string();

    Ok(response)
}
