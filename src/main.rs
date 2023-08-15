use anyhow::{Error, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use tokio::net::UdpSocket;

use ac_ffmpeg::{
    codec::{video::VideoDecoder, Decoder},
    format::{
        demuxer::{Demuxer, DemuxerWithStreamInfo},
        io::IO,
    },
};

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

    // Indicate in the Transport heading whether you want TCP/UDP
    // With this camera it seems when TCP is chosen, then the
    // server will NOT respond with a port number. I guess this
    // means that it uses existing TCP connection to send RTP?
    // When UDP is chosen, a port is provided in response. With
    // this camera (Topodome) choosing UDP provided a port in
    // the response at 6600.
    session.transport = "Transport: RTP/AVP/TCP;unicast;client_port=4588-4589\r\n".to_string();
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
        // let stream = TcpStream::connect("192.168.86.138:6600")?;
        // stream.read(&mut [0; 128])?;

        let io = IO::from_read_stream(session.stream);

        let mut demuxer = Demuxer::builder()
            .build(io)?
            .find_stream_info(None)
            .map_err(|(_, err)| err)?;

        let (stream_index, (stream, _)) = demuxer
            .streams()
            .iter()
            .map(|stream| (stream, stream.codec_parameters()))
            .enumerate()
            .find(|(_, (_, params))| params.is_video_codec())
            .unwrap();

        let mut decoder = VideoDecoder::from_stream(stream)?.build()?;

        // process data
        loop {
            if let Some(packet) = demuxer.take()? {
                if packet.stream_index() != stream_index {
                    continue;
                }

                decoder.push(packet)?;

                while let Some(frame) = decoder.take()? {
                    println!("{}", frame.width());
                }
            }
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
