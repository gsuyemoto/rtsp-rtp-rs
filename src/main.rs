use anyhow::{Error, Result};
use std::io::{Read, Write};
use std::net::TcpStream;

// #[derive(Debug, Deserialize)]
// struct SessionDescription {
//     #[serde(rename = "v")]
//     version: u8,

//     #[serde(rename = "o")]
//     owner: String,

//     #[serde(rename = "s")]
//     session_name: String,

//     #[serde(rename = "c")]
//     connection: String,

//     #[serde(rename = "t")]
//     time: String,

//     #[serde(rename = "a")]
//     session_attribute_01: String,

//     #[serde(rename = "a")]
//     session_attribute_02: String,

//     #[serde(rename = "m")]
//     media_description: String,

//     #[serde(rename = "a")]
//     media_attribute_01: String,

//     #[serde(rename = "a")]
//     media_attribute_02: String,
// }

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

    session.transport = "Transport: RTP/AVP;unicast;client_port=4588-4589\r\n".to_string();
    session.track = "trackID=0".to_string();

    let response = send_basic_rtsp_request(&mut session, "SETUP").await?;
    println!("SETUP: \n{response}");

    session.name = "Session: ".to_string();

    let response = send_basic_rtsp_request(&mut session, "PLAY").await?;
    println!("PLAY: \n{response}");

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
