use anyhow::{Error, Result};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};

pub enum Methods {
    Options,
    Describe,
    Setup,
    Play,
    Teardown,
}

#[derive(Debug)]
pub struct Session {
    cseq: u32,
    server_addr: String,
    stream: TcpStream,
    transport: String,
    track: String,
    buf_size: usize,
    id: String,
}

impl Session {
    pub fn new(server_addr: String) -> Result<Self, Error> {
        let tcp_stream = TcpStream::connect(&server_addr)?;

        // Indicate in the Transport heading whether you want TCP/UDP
        // With this camera it seems when TCP is chosen, then the
        // server will NOT respond with a port number. I guess this
        // means that it uses existing TCP connection to send RTP?
        // When UDP is chosen, a port is provided in response. With
        // this camera (Topodome) choosing UDP provided a port in
        // the response at 6600.

        // I think you need to append the token received in SETUP
        // response here. With my test camera, it was not necessary
        // and without the token, I still received 200 OK

        Ok(Session {
            server_addr,
            stream: tcp_stream,
            transport: String::new(),
            track: String::new(),
            id: String::new(),
            cseq: 1,
            buf_size: 1024,
        })
    }

    #[rustfmt::skip]
    pub async fn send(&mut self, method_in: Methods) -> Result<String, Error> {
        let method_str = match method_in {
            Methods::Options     => "OPTIONS",
            Methods::Describe    => "DESCRIBE",
            Methods::Setup       => "SETUP",
            Methods::Play        => "PLAY",
            Methods::Teardown    => "TEARDOWN",
        };

        // Need to add headers to request for different methods
        match method_in {
            Methods::Options     => (),
            Methods::Describe    => (),
            Methods::Setup       => {
                                        self.transport =
                                            "Transport: RTP/AVP/UDP;unicast;client_port=4588-4589\r\n".to_string();
                                        self.track = "/trackID=0\r\n".to_string();
                                    }
            Methods::Play        => {
                                        self.transport = String::new();
                                        self.track = String::new();
                                    }
            Methods::Teardown    => (),
        }

        let request = format!(
            "{} {}{} RTSP/1.0\r\nCSeq: {}\r\n{}{}\r\n",
            method_str, 
            self.server_addr, 
            self.track, 
            self.cseq, 
            self.transport, 
            self.id,
        );

        // let mut buffer = Vec::with_capacity(self.buf_size);
        let mut buffer = [0u8; 1024];

        // Send command with proper headers
        // every command must provide cseq
        // which is incremented sequence as a header
        self.stream.write(request.as_bytes())?;
        let resp_size = self.stream.read(&mut buffer)?;
        self.cseq += 1;

        println!("Response bytes: {resp_size}"); 

        // Some responses come with specially formatted
        // data that depends on type of command sent
        match method_in {
            Methods::Options     => (),
            Methods::Describe    => return Ok(self.get_sdp(buffer)),
            Methods::Setup       => return Ok(self.get_id(buffer)),
            Methods::Play        => (),
            Methods::Teardown    => (),
        }

        Ok(parse_response(buffer))
    }

    pub fn stop(&mut self) -> String {
        let mut result = String::new();

        let request = format!(
            "TEARDOWN {} RTSP/1.0\r\nCSeq: {}\r\n{}\r\n",
            self.server_addr, self.cseq, self.id,
        );

        // let mut buffer = Vec::with_capacity(self.buf_size);
        let mut buffer = [0u8; 1024];

        match self.stream.write(request.as_bytes()) {
            Ok(_) => result.push_str("Write Ok\n"),
            Err(e) => return format!("Write Error: {e}"),
        }

        match self.stream.read(&mut buffer) {
            Ok(_) => result.push_str(&parse_response(buffer)),
            Err(e) => return format!("Read Error: {e}"),
        }

        match self.stream.shutdown(Shutdown::Both) {
            Ok(_) => result.push_str("Shutdown Ok"),
            Err(e) => return format!("Shutdown Error: {e}"),
        }

        result
    }

    fn get_sdp(&mut self, buf: [u8; 1024]) -> String {
        // SDP data begins after \r\n\r\n
        let response = parse_response(buf);
        let (headers, sdp) = response.split_once("\r\n\r\n").unwrap();
        
        let sdp_fields = sdp.lines();
        
        headers.to_owned()
    }

    fn get_id(&mut self, buf: [u8; 1024]) -> String {
        let response = parse_response(buf);
        let resp_headers = response.lines();

        let session_id = resp_headers
            .into_iter()
            .filter(|line| line.contains("Session"))
            .map(|line| line.split(|c| c == ':' || c == ';').collect::<Vec<&str>>())
            .map(|v| v[1])
            .collect::<String>();

        // println!("Session id: {session_id}");
        self.id = format!("Session: {session_id}");

        response
    }
}

fn parse_response(buf: [u8; 1024]) -> String {
    // String::from_utf8(buf.clone()).unwrap_or("Error parsing response".to_string())
    (*String::from_utf8_lossy(&buf)).to_string()
}