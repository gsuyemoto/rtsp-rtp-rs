use anyhow::{Error, Result};
use std::io::{Read, Write};
use std::net::TcpStream;

#[derive(Debug)]
pub struct Session {
    cseq: u32,
    server_addr: String,
    stream: TcpStream,
    transport: String,
    track: String,
    name: String,
}

pub enum Methods {
    Options,
    Describe,
    Setup,
    Play,
}

impl Session {
    pub fn new(server_addr: String, stream: TcpStream) -> Self {
        Session {
            server_addr,
            stream,
            transport: String::new(),
            track: String::new(),
            name: String::new(),
            cseq: 1,
        }
    }

    async fn send_basic_rtsp_request(
        sess: &mut Session,
        method_in: Methods,
    ) -> Result<String, Error> {
        #[rustfmt::skip]
        let method = match method_in {
            Methods::Options     => "OPTIONS",
            Methods::Describe    => "DESCRIBE",
            Methods::Setup       => "SETUP",
            Methods::Play        => "PLAY",
        };

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
}
